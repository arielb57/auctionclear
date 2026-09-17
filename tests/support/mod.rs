#![allow(dead_code)]

pub mod reference;

use auctionclear::{Limit, NoTradeReason, Order, Outcome, Side};

fn crosses(o: &Order, price: i64) -> bool {
    o.executable_at(price)
}

/// Checks every book-level invariant of an outcome without trusting either
/// implementation's price-choice logic.
pub fn check_invariants(
    orders: &[Order],
    outcome: &Outcome,
    reference: Option<i64>,
) -> Result<(), String> {
    let candidate_prices = reference::candidate_prices(orders, reference);
    let c = match outcome {
        Outcome::NoTrade(NoTradeReason::NoCross) => {
            for p in candidate_prices {
                let m = reference::metrics(orders, p);
                if m.volume > 0 {
                    return Err(format!(
                        "reported no cross, but {} shares execute at {p}",
                        m.volume
                    ));
                }
            }
            return Ok(());
        }
        Outcome::NoTrade(NoTradeReason::NoReferencePrice) => {
            let only_market = orders.iter().all(|o| o.limit == Limit::Market);
            let both = [Side::Buy, Side::Sell]
                .iter()
                .all(|s| orders.iter().any(|o| o.side == *s));
            return if only_market && both && reference.is_none() {
                Ok(())
            } else {
                Err("NoReferencePrice reported for a book that has a price".into())
            };
        }
        Outcome::Cleared(c) => c,
    };

    if c.fills.len() != orders.len() {
        return Err("fills length differs from order count".into());
    }
    let price = c.price;
    let mut bought = 0u128;
    let mut sold = 0u128;
    for (o, &f) in orders.iter().zip(&c.fills) {
        if f > o.qty {
            return Err(format!("order {} filled {f} > qty {}", o.id, o.qty));
        }
        if f > 0 && !crosses(o, price) {
            return Err(format!("order {} filled at {price} beyond its limit", o.id));
        }
        if o.limit_strictly_better(price) && f != o.qty {
            return Err(format!(
                "order {} priced through {price} filled only {f}/{}",
                o.id, o.qty
            ));
        }
        match o.side {
            Side::Buy => bought += f as u128,
            Side::Sell => sold += f as u128,
        }
    }
    if bought != sold || bought != c.volume {
        return Err(format!(
            "bought {bought}, sold {sold}, reported volume {}",
            c.volume
        ));
    }
    if c.volume == 0 {
        return Err("cleared with zero volume".into());
    }

    let at = reference::metrics(orders, price);
    if at.volume != c.volume || at.imbalance != c.imbalance {
        return Err(format!(
            "reported volume/imbalance {}/{} but the book gives {}/{} at {price}",
            c.volume, c.imbalance, at.volume, at.imbalance
        ));
    }
    for p in candidate_prices {
        let m = reference::metrics(orders, p);
        if m.volume > c.volume {
            return Err(format!(
                "price {p} executes {} > {} at the clearing price {price}",
                m.volume, c.volume
            ));
        }
    }

    // Residual book must not cross. Market orders rank as ±infinity.
    let rank = |o: &Order| match o.limit {
        Limit::Market if o.side == Side::Buy => i128::MAX,
        Limit::Market => i128::MIN,
        Limit::Price(p) => p as i128,
    };
    let residual = |side: Side| {
        orders
            .iter()
            .zip(&c.fills)
            .filter(move |(o, &f)| o.side == side && f < o.qty)
    };
    let best_bid = residual(Side::Buy).map(|(o, _)| rank(o)).max();
    let best_ask = residual(Side::Sell).map(|(o, _)| rank(o)).min();
    if let (Some(b), Some(a)) = (best_bid, best_ask) {
        if b >= a {
            return Err(format!("residual book is crossed: bid {b} >= ask {a}"));
        }
    }

    for side in [Side::Buy, Side::Sell] {
        let exec: Vec<(usize, &Order)> = orders
            .iter()
            .enumerate()
            .filter(|(_, o)| o.side == side && crosses(o, price))
            .collect();
        let partial_market = exec
            .iter()
            .any(|(i, o)| o.limit == Limit::Market && c.fills[*i] < o.qty);
        if partial_market
            && exec
                .iter()
                .any(|(i, o)| o.limit != Limit::Market && c.fills[*i] > 0)
        {
            return Err(format!(
                "{side:?} limit order filled while a market order was short"
            ));
        }
        // Within the market group and within the at-price group, fills must
        // follow time priority with at most one partial fill.
        for group_is_market in [true, false] {
            let mut group: Vec<&(usize, &Order)> = exec
                .iter()
                .filter(|(_, o)| {
                    (o.limit == Limit::Market) == group_is_market && !o.limit_strictly_better(price)
                })
                .collect();
            group.sort_by_key(|(i, o)| (o.time, *i));
            let mut seen_short = false;
            for (i, o) in group {
                let f = c.fills[*i];
                if seen_short && f > 0 {
                    return Err(format!(
                        "order {} filled after an earlier order was short",
                        o.id
                    ));
                }
                if f < o.qty {
                    seen_short = true;
                }
            }
        }
    }
    let short = |side: Side| {
        orders
            .iter()
            .zip(&c.fills)
            .any(|(o, &f)| o.side == side && crosses(o, price) && f < o.qty)
    };
    if short(Side::Buy) && short(Side::Sell) {
        return Err("executable orders left unfilled on both sides".into());
    }
    Ok(())
}
