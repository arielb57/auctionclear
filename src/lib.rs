//! Call-auction uncrossing with venue-specific tie-break chains.
//!
//! ```
//! use auctionclear::{Book, Order, Outcome, Side, Venue};
//!
//! let book = Book::new(vec![
//!     Order::limit(1, Side::Buy, 101, 300, 0),
//!     Order::limit(2, Side::Buy, 100, 200, 1),
//!     Order::limit(3, Side::Sell, 99, 400, 2),
//!     Order::market(4, Side::Sell, 50, 3),
//! ]).unwrap();
//! let Outcome::Cleared(c) = book.uncross(&Venue::Xetra.policy(), Some(100)).unwrap() else {
//!     panic!("book crosses");
//! };
//! assert_eq!((c.price, c.volume), (100, 450));
//! assert_eq!(c.fills, vec![300, 150, 400, 50]);
//! ```

pub mod book;
pub mod csv;
pub mod gen;
pub mod order;
pub mod policy;

pub use book::{Book, BookError, Clearing, Level, NoTradeReason, Outcome, UncrossError};
pub use order::{Limit, Order, Side};
pub use policy::{Candidate, Policy, PolicyError, Step, Venue};
