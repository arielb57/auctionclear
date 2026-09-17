use std::fmt;

use crate::order::{Limit, Order, Side};
use crate::policy::{Candidate, Policy, Step};

/// Aggregated limit quantity at one distinct price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub price: i64,
    pub bid: u128,
    pub ask: u128,
}

/// The result of uncrossing a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Cleared(Clearing),
    NoTrade(NoTradeReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clearing {
    pub price: i64,
    pub volume: u128,
    /// `demand - supply` at the clearing price.
    pub imbalance: i128,
    /// Filled quantity per order, in book order.
    pub fills: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoTradeReason {
    /// No price has positive executable volume.
    NoCross,
    /// Both sides hold only market orders and no reference price was given.
    NoReferencePrice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncrossError {
    /// A filter step removed every candidate.
    EmptyAfter(Step),
    /// The chain chose a price at which an order priced through it could not
    /// fill completely. Presets never do this; custom chains can.
    Unclearable { price: i64 },
}

impl fmt::Display for UncrossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UncrossError::EmptyAfter(s) => write!(f, "step {s} left no candidate price"),
            UncrossError::Unclearable { price } => write!(
                f,
                "the chain chose {price}, where an order priced through the auction price cannot fill completely"
            ),
        }
    }
}

impl std::error::Error for UncrossError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookError {
    ZeroQuantity { id: u64 },
}

impl fmt::Display for BookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookError::ZeroQuantity { id } => write!(f, "order {id} has zero quantity"),
        }
    }
}

impl std::error::Error for BookError {}

/// An auction book with its price levels aggregated.
#[derive(Debug, Clone)]
pub struct Book {
    orders: Vec<Order>,
    levels: Vec<Level>,
    market_bid: u128,
    market_ask: u128,
    /// `bid_from[i]` = limit bid quantity at levels `i..`; length `levels + 1`.
    bid_from: Vec<u128>,
    /// `ask_upto[i]` = limit ask quantity at levels `..i`; length `levels + 1`.
    ask_upto: Vec<u128>,
}

impl Book {
    pub fn new(orders: Vec<Order>) -> Result<Book, BookError> {
        if let Some(o) = orders.iter().find(|o| o.qty == 0) {
            return Err(BookError::ZeroQuantity { id: o.id });
        }
        let mut market_bid = 0u128;
        let mut market_ask = 0u128;
        let mut limits: Vec<(i64, Side, u64)> = Vec::with_capacity(orders.len());
        for o in &orders {
            match (o.limit, o.side) {
                (Limit::Market, Side::Buy) => market_bid += o.qty as u128,
                (Limit::Market, Side::Sell) => market_ask += o.qty as u128,
                (Limit::Price(p), side) => limits.push((p, side, o.qty)),
            }
        }
        limits.sort_unstable_by_key(|&(p, _, _)| p);

        let mut levels: Vec<Level> = Vec::new();
        for (p, side, q) in limits {
            if levels.last().map(|l| l.price) != Some(p) {
                levels.push(Level {
                    price: p,
                    bid: 0,
                    ask: 0,
                });
            }
            let level = levels.last_mut().expect("pushed above");
            match side {
                Side::Buy => level.bid += q as u128,
                Side::Sell => level.ask += q as u128,
            }
        }

        let n = levels.len();
        let mut bid_from = vec![0u128; n + 1];
        let mut ask_upto = vec![0u128; n + 1];
        for i in (0..n).rev() {
            bid_from[i] = bid_from[i + 1] + levels[i].bid;
        }
        for i in 0..n {
            ask_upto[i + 1] = ask_upto[i] + levels[i].ask;
        }

        Ok(Book {
            orders,
            levels,
            market_bid,
            market_ask,
            bid_from,
            ask_upto,
        })
    }

    pub fn orders(&self) -> &[Order] {
        &self.orders
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// Metrics at `price`, where `at_or_above` is the index of the first level
    /// `>= price` and `above` the index of the first level `> price`.
    fn metrics(&self, price: i64, at_or_above: usize, above: usize) -> Candidate {
        let demand = self.market_bid + self.bid_from[at_or_above];
        let supply = self.market_ask + self.ask_upto[above];
        let volume = demand.min(supply);
        let bids_through = self.bid_from[above];
        let asks_through = self.ask_upto[at_or_above];
        // Market orders outrank every limit order on their side, so limit
        // orders priced through the candidate fill completely only if the
        // market orders ahead of them leave room.
        let side_ok = |through: u128, market: u128| through == 0 || market + through <= volume;
        Candidate {
            price,
            demand,
            supply,
            volume,
            imbalance: demand as i128 - supply as i128,
            eligible: side_ok(bids_through, self.market_bid)
                && side_ok(asks_through, self.market_ask),
        }
    }

    /// Volume, imbalance and eligibility at an arbitrary price. O(log levels).
    pub fn metrics_at(&self, price: i64) -> Candidate {
        let at_or_above = self.levels.partition_point(|l| l.price < price);
        let above = self.levels.partition_point(|l| l.price <= price);
        self.metrics(price, at_or_above, above)
    }

    /// Candidate prices in ascending order: every distinct limit price, or,
    /// if the book holds no limit orders, the reference price alone.
    /// One pass over the levels.
    pub fn candidates(&self, reference: Option<i64>) -> Vec<Candidate> {
        if self.levels.is_empty() {
            return reference.map(|r| self.metrics_at(r)).into_iter().collect();
        }
        (0..self.levels.len())
            .map(|i| self.metrics(self.levels[i].price, i, i + 1))
            .collect()
    }

    /// Determines the auction price with `policy` and allocates fills by
    /// price-time priority.
    pub fn uncross(
        &self,
        policy: &Policy,
        reference: Option<i64>,
    ) -> Result<Outcome, UncrossError> {
        let candidates = self.candidates(reference);
        if candidates.is_empty() {
            let reason = if self.market_bid > 0 && self.market_ask > 0 {
                NoTradeReason::NoReferencePrice
            } else {
                NoTradeReason::NoCross
            };
            return Ok(Outcome::NoTrade(reason));
        }
        if candidates.iter().all(|c| c.volume == 0) {
            return Ok(Outcome::NoTrade(NoTradeReason::NoCross));
        }
        let price = policy
            .choose(&candidates, reference)
            .map_err(UncrossError::EmptyAfter)?;
        let at = self.metrics_at(price);
        if !at.eligible || at.volume == 0 {
            return Err(UncrossError::Unclearable { price });
        }
        let mut fills = vec![0u64; self.orders.len()];
        self.allocate(Side::Buy, price, at.volume, &mut fills);
        self.allocate(Side::Sell, price, at.volume, &mut fills);
        Ok(Outcome::Cleared(Clearing {
            price,
            volume: at.volume,
            imbalance: at.imbalance,
            fills,
        }))
    }

    /// Fills `volume` on one side at `price`. Priority groups are market
    /// orders, then limits priced through `price`, then limits at `price`.
    /// Whole groups fill until one does not fit; only that group is sorted by
    /// time and rationed.
    fn allocate(&self, side: Side, price: i64, volume: u128, fills: &mut [u64]) {
        let at_or_above = self.levels.partition_point(|l| l.price < price);
        let above = self.levels.partition_point(|l| l.price <= price);
        let (market, through, at) = match side {
            Side::Buy => (
                self.market_bid,
                self.bid_from[above],
                self.bid_from[at_or_above] - self.bid_from[above],
            ),
            Side::Sell => (
                self.market_ask,
                self.ask_upto[at_or_above],
                self.ask_upto[above] - self.ask_upto[at_or_above],
            ),
        };

        #[derive(Clone, Copy, PartialEq)]
        enum Group {
            Market,
            Through,
            At,
        }
        let mut remaining = volume;
        let mut full = [false; 3];
        let mut rationed: Option<(Group, u128)> = None;
        for (slot, (group, size)) in [
            (Group::Market, market),
            (Group::Through, through),
            (Group::At, at),
        ]
        .into_iter()
        .enumerate()
        {
            if size <= remaining {
                full[slot] = true;
                remaining -= size;
            } else {
                if remaining > 0 {
                    rationed = Some((group, remaining));
                }
                break;
            }
        }

        let group_of = |o: &Order| -> Option<Group> {
            if o.side != side || !o.executable_at(price) {
                None
            } else if o.limit == Limit::Market {
                Some(Group::Market)
            } else if o.limit_strictly_better(price) {
                Some(Group::Through)
            } else {
                Some(Group::At)
            }
        };
        let slot = |g: Group| match g {
            Group::Market => 0,
            Group::Through => 1,
            Group::At => 2,
        };

        let mut queue: Vec<usize> = Vec::new();
        for (i, o) in self.orders.iter().enumerate() {
            let Some(g) = group_of(o) else { continue };
            if full[slot(g)] {
                fills[i] = o.qty;
            } else if rationed.is_some_and(|(rg, _)| rg == g) {
                queue.push(i);
            }
        }
        if let Some((_, mut left)) = rationed {
            let price_rank = |o: &Order| match (o.side, o.limit) {
                (_, Limit::Market) => 0,
                (Side::Buy, Limit::Price(p)) => -(p as i128),
                (Side::Sell, Limit::Price(p)) => p as i128,
            };
            queue.sort_unstable_by_key(|&i| (price_rank(&self.orders[i]), self.orders[i].time, i));
            for i in queue {
                if left == 0 {
                    break;
                }
                let take = left.min(self.orders[i].qty as u128);
                fills[i] = take as u64;
                left -= take;
            }
        }
    }
}
