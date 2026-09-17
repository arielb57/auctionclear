//! Price collars: the rule that stops an auction printing a price at all.
//!
//! Everything else in this crate answers "which price?". A real venue asks a
//! prior question — *should this print?* If the price the book would clear at
//! sits too far from a reference, no major exchange simply prints it. Xetra
//! calls the result a volatility interruption, Euronext a reservation, the US
//! markets an extension under the limit up-limit down bands; in each case the
//! auction is prolonged rather than uncrossed, and the imbalance is published
//! so someone can react to it.
//!
//! That makes the collar a third outcome, not a filter. Dropping the offending
//! price from the candidate set would be wrong twice over: the auction would
//! print at some other, worse price, and the caller would never learn that the
//! venue had stepped in. A collar breach is a decision *not* to trade yet, and
//! it has to look like one.
//!
//! The band here is symmetric and static: a fixed distance either side of a
//! reference price, in basis points. Real books layer a dynamic band against
//! the last trade on top of a static one against the previous close, and the
//! widths vary by instrument, by time of day and by how many extensions have
//! already happened. This models the shape, not any venue's table.

use std::fmt;

/// A static price band around a reference price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Collar {
    /// Half-width of the band, in basis points of the reference price.
    pub max_bps: u32,
}

/// How far an indicative price sat from the reference, and by how much it
/// broke the band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breach {
    pub indicative: i64,
    pub reference: i64,
    pub deviation_bps: u64,
    pub limit_bps: u32,
}

impl fmt::Display for Breach {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is {} bps from the reference {}, past the {} bps collar",
            self.indicative, self.deviation_bps, self.reference, self.limit_bps
        )
    }
}

impl Collar {
    pub fn new(max_bps: u32) -> Self {
        Collar { max_bps }
    }

    /// Distance from `reference` to `price`, in basis points, rounded down.
    ///
    /// Rounding down is the permissive direction: a price exactly on the band
    /// clears. A venue that wants the other reading moves the band by one.
    /// The arithmetic is in `i128` because a price in ticks times 10,000
    /// overflows `i64` sooner than a book's range suggests.
    pub fn deviation_bps(price: i64, reference: i64) -> Option<u64> {
        if reference <= 0 {
            return None;
        }
        let diff = (i128::from(price) - i128::from(reference)).unsigned_abs();
        Some(
            u64::try_from(diff * 10_000 / u128::from(reference.unsigned_abs())).unwrap_or(u64::MAX),
        )
    }

    /// The breach, if `price` sits outside the band around `reference`.
    ///
    /// A collar needs something to measure against: with no reference price
    /// there is no band, and the auction prints. That is the honest reading —
    /// a venue without a previous close does not invent one — and it is why
    /// this returns `None` rather than refusing to trade.
    pub fn check(self, price: i64, reference: Option<i64>) -> Option<Breach> {
        let reference = reference?;
        let deviation_bps = Self::deviation_bps(price, reference)?;
        if deviation_bps <= u64::from(self.max_bps) {
            return None;
        }
        Some(Breach {
            indicative: price,
            reference,
            deviation_bps,
            limit_bps: self.max_bps,
        })
    }
}
