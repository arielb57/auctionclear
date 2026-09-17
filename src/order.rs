/// Which side of the book an order is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

/// An order's price instruction, in integer ticks.
///
/// A market buy behaves as a bid at +∞ and a market sell as an ask at −∞:
/// it is executable at every candidate price and has priority over every
/// limit order on its side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Limit {
    Market,
    Price(i64),
}

/// One auction order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Order {
    pub id: u64,
    pub side: Side,
    pub limit: Limit,
    /// Shares. Must be non-zero.
    pub qty: u64,
    /// Entry time. Earlier is higher priority; equal times fall back to the
    /// order's position in the book.
    pub time: u64,
}

impl Order {
    pub fn limit(id: u64, side: Side, price: i64, qty: u64, time: u64) -> Self {
        Order {
            id,
            side,
            limit: Limit::Price(price),
            qty,
            time,
        }
    }

    pub fn market(id: u64, side: Side, qty: u64, time: u64) -> Self {
        Order {
            id,
            side,
            limit: Limit::Market,
            qty,
            time,
        }
    }

    /// True if this order may trade at `price`.
    pub fn executable_at(&self, price: i64) -> bool {
        match (self.side, self.limit) {
            (_, Limit::Market) => true,
            (Side::Buy, Limit::Price(p)) => p >= price,
            (Side::Sell, Limit::Price(p)) => p <= price,
        }
    }

    /// True if this is a limit order priced strictly through `price`
    /// (a bid above it or an ask below it).
    pub fn limit_strictly_better(&self, price: i64) -> bool {
        match (self.side, self.limit) {
            (_, Limit::Market) => false,
            (Side::Buy, Limit::Price(p)) => p > price,
            (Side::Sell, Limit::Price(p)) => p < price,
        }
    }
}
