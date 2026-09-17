//! Brute-force reference uncross. Deliberately naive and independent of the
//! crate's sweep: every candidate price rescans every order, the chain works
//! on explicit lists, and fills come from sorting the whole side by priority.
//! O(levels × orders). Used only by tests and the benchmark.

use std::collections::BTreeSet;

use auctionclear::{Clearing, Limit, NoTradeReason, Order, Outcome, Side, Step, UncrossError};

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub price: i64,
    pub volume: u128,
    pub imbalance: i128,
    pub eligible: bool,
}

fn crosses(o: &Order, price: i64) -> bool {
    match (o.side, o.limit) {
        (_, Limit::Market) => true,
        (Side::Buy, Limit::Price(p)) => p >= price,
        (Side::Sell, Limit::Price(p)) => p <= price,
    }
}

fn through(o: &Order, price: i64) -> bool {
    match (o.side, o.limit) {
        (_, Limit::Market) => false,
        (Side::Buy, Limit::Price(p)) => p > price,
        (Side::Sell, Limit::Price(p)) => p < price,
    }
}

pub fn metrics(orders: &[Order], price: i64) -> Metrics {
    let mut demand = 0u128;
    let mut supply = 0u128;
    for o in orders.iter().filter(|o| crosses(o, price)) {
        match o.side {
            Side::Buy => demand += o.qty as u128,
            Side::Sell => supply += o.qty as u128,
        }
    }
    let volume = demand.min(supply);
    // Eligible iff price-time allocation at this price fills every order
    // priced through it: simulate the allocation and look.
    let fills = allocate(orders, price, volume);
    let eligible = orders
        .iter()
        .zip(&fills)
        .all(|(o, &f)| !through(o, price) || f == o.qty);
    Metrics {
        price,
        volume,
        imbalance: demand as i128 - supply as i128,
        eligible,
    }
}

pub fn candidate_prices(orders: &[Order], reference: Option<i64>) -> Vec<i64> {
    let prices: BTreeSet<i64> = orders
        .iter()
        .filter_map(|o| match o.limit {
            Limit::Price(p) => Some(p),
            Limit::Market => None,
        })
        .collect();
    if prices.is_empty() {
        reference.into_iter().collect()
    } else {
        prices.into_iter().collect()
    }
}

fn allocate(orders: &[Order], price: i64, volume: u128) -> Vec<u64> {
    let mut fills = vec![0u64; orders.len()];
    for side in [Side::Buy, Side::Sell] {
        let mut queue: Vec<usize> = (0..orders.len())
            .filter(|&i| orders[i].side == side && crosses(&orders[i], price))
            .collect();
        queue.sort_by(|&a, &b| {
            let (x, y) = (&orders[a], &orders[b]);
            let rank = |o: &Order| match (o.side, o.limit) {
                (_, Limit::Market) => i128::MIN,
                (Side::Buy, Limit::Price(p)) => -(p as i128),
                (Side::Sell, Limit::Price(p)) => p as i128,
            };
            rank(x)
                .cmp(&rank(y))
                .then(x.time.cmp(&y.time))
                .then(a.cmp(&b))
        });
        let mut left = volume;
        for i in queue {
            let take = left.min(orders[i].qty as u128);
            fills[i] = take as u64;
            left -= take;
        }
    }
    fills
}

pub fn uncross(
    orders: &[Order],
    steps: &[Step],
    reference: Option<i64>,
) -> Result<Outcome, UncrossError> {
    let prices = candidate_prices(orders, reference);
    let has_market = |s: Side| {
        orders
            .iter()
            .any(|o| o.side == s && o.limit == Limit::Market)
    };
    if prices.is_empty() {
        return Ok(Outcome::NoTrade(
            if has_market(Side::Buy) && has_market(Side::Sell) {
                NoTradeReason::NoReferencePrice
            } else {
                NoTradeReason::NoCross
            },
        ));
    }
    let mut set: Vec<Metrics> = prices.iter().map(|&p| metrics(orders, p)).collect();
    if set.iter().all(|m| m.volume == 0) {
        return Ok(Outcome::NoTrade(NoTradeReason::NoCross));
    }

    let mut price = None;
    for &step in steps {
        match step {
            Step::MaxVolume => {
                let best = set.iter().map(|m| m.volume).max().unwrap();
                set.retain(|m| m.volume == best);
            }
            Step::Eligible => set.retain(|m| m.eligible),
            Step::MinAbsImbalance => {
                let best = set.iter().map(|m| m.imbalance.abs()).min().unwrap();
                set.retain(|m| m.imbalance.abs() == best);
            }
            Step::ImbalanceSide => {
                if set.iter().all(|m| m.imbalance > 0) {
                    let hi = set.iter().map(|m| m.price).max().unwrap();
                    set.retain(|m| m.price == hi);
                } else if set.iter().all(|m| m.imbalance < 0) {
                    let lo = set.iter().map(|m| m.price).min().unwrap();
                    set.retain(|m| m.price == lo);
                }
            }
            Step::NearestReference => {
                if let Some(r) = reference {
                    let best = set
                        .iter()
                        .map(|m| (m.price as i128 - r as i128).abs())
                        .min()
                        .unwrap();
                    set.retain(|m| (m.price as i128 - r as i128).abs() == best);
                }
            }
            Step::Midpoint | Step::ClampReference | Step::Lowest | Step::Highest => {
                let lo = set.iter().map(|m| m.price).min().unwrap() as i128;
                let hi = set.iter().map(|m| m.price).max().unwrap() as i128;
                let mid = if (lo + hi) % 2 == 0 || lo + hi >= 0 {
                    (lo + hi) / 2
                } else {
                    (lo + hi) / 2 - 1
                };
                price = Some(match (step, reference) {
                    (Step::Lowest, _) => lo,
                    (Step::Highest, _) => hi,
                    (Step::ClampReference, Some(r)) => (r as i128).max(lo).min(hi),
                    _ => mid,
                } as i64);
            }
        }
        if set.is_empty() {
            return Err(UncrossError::EmptyAfter(step));
        }
    }
    let price = price.expect("chains end with a terminal step");
    let at = metrics(orders, price);
    if !at.eligible || at.volume == 0 {
        return Err(UncrossError::Unclearable { price });
    }
    Ok(Outcome::Cleared(Clearing {
        price,
        volume: at.volume,
        imbalance: at.imbalance,
        fills: allocate(orders, price, at.volume),
    }))
}
