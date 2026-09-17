//! Random auction books, including distributions built to tie on volume and
//! imbalance.

use crate::order::{Order, Side};

/// SplitMix64: small, seedable and good enough for test data.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`. `n` must be non-zero.
    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi as i128 - lo as i128 + 1) as u128;
        if span > u64::MAX as u128 {
            return self.next_u64() as i64;
        }
        (lo as i128 + self.below(span as u64) as i128) as i64
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Shape of a generated book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Prices spread over roughly half as many ticks as orders, quantities 1..=100.
    Uniform,
    /// A handful of ticks, quantities 1..=3, mirrored bid/ask pairs and
    /// repeated timestamps: most books tie on volume, many on imbalance too.
    TieHeavy,
    /// About half the orders are market orders, over very few ticks.
    MarketHeavy,
    /// Quantities within a few shares of `u64::MAX` and prices at the ends
    /// of the `i64` range.
    Extreme,
    /// A random choice of the other profiles per book, plus books with an
    /// empty side or a single order.
    Mixed,
}

impl Profile {
    pub const ALL: [Profile; 5] = [
        Profile::Uniform,
        Profile::TieHeavy,
        Profile::MarketHeavy,
        Profile::Extreme,
        Profile::Mixed,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Profile::Uniform => "uniform",
            Profile::TieHeavy => "tie-heavy",
            Profile::MarketHeavy => "market-heavy",
            Profile::Extreme => "extreme",
            Profile::Mixed => "mixed",
        }
    }

    pub fn from_name(name: &str) -> Option<Profile> {
        Profile::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
    }
}

/// A generated book and the reference price to uncross it with.
#[derive(Debug, Clone)]
pub struct Generated {
    pub orders: Vec<Order>,
    pub reference: Option<i64>,
}

fn side(rng: &mut Rng) -> Side {
    if rng.chance(50) {
        Side::Buy
    } else {
        Side::Sell
    }
}

/// Generates a book of about `n` orders (tie-heavy books may add mirrored pairs).
pub fn generate(profile: Profile, n: usize, rng: &mut Rng) -> Generated {
    match profile {
        Profile::Uniform => {
            let spread = (n as i64 / 2).max(2);
            let base = 1_000;
            let orders = (0..n)
                .map(|i| {
                    let s = side(rng);
                    let qty = rng.range(1, 100) as u64;
                    let time = rng.below(2 * n as u64 + 1);
                    if rng.chance(5) {
                        Order::market(i as u64, s, qty, time)
                    } else {
                        Order::limit(i as u64, s, base + rng.range(-spread, spread), qty, time)
                    }
                })
                .collect();
            let reference = rng
                .chance(90)
                .then(|| base + rng.range(-spread - 2, spread + 2));
            Generated { orders, reference }
        }
        Profile::TieHeavy => {
            let base = 100;
            let width = rng.range(0, 4);
            let mut orders = Vec::with_capacity(n + 2);
            while orders.len() < n {
                let id = orders.len() as u64;
                let time = rng.below(4);
                let qty = rng.range(1, 3) as u64;
                let roll = rng.below(100);
                if roll < 25 && orders.len() + 2 <= n + 1 {
                    // A bid one or more ticks above an equal ask adds an ask-only
                    // level below a bid-only level: both prices then have the same
                    // volume and the same imbalance.
                    let p = base + rng.range(-width, width);
                    let gap = rng.range(1, 2);
                    orders.push(Order::limit(id, Side::Buy, p + gap, qty, time));
                    orders.push(Order::limit(id + 1, Side::Sell, p, qty, rng.below(4)));
                } else if roll < 35 {
                    orders.push(Order::market(id, side(rng), qty, time));
                } else {
                    orders.push(Order::limit(
                        id,
                        side(rng),
                        base + rng.range(-width, width),
                        qty,
                        time,
                    ));
                }
            }
            let reference = rng
                .chance(85)
                .then(|| base + rng.range(-width - 3, width + 3));
            Generated { orders, reference }
        }
        Profile::MarketHeavy => {
            let base = 50;
            let orders = (0..n)
                .map(|i| {
                    let s = side(rng);
                    let qty = rng.range(1, 10) as u64;
                    let time = rng.below(n as u64 + 1);
                    if rng.chance(50) {
                        Order::market(i as u64, s, qty, time)
                    } else {
                        Order::limit(i as u64, s, base + rng.range(-2, 2), qty, time)
                    }
                })
                .collect();
            let reference = rng.chance(80).then(|| base + rng.range(-4, 4));
            Generated { orders, reference }
        }
        Profile::Extreme => {
            let low_end = rng.chance(50);
            let price = |rng: &mut Rng| {
                if low_end {
                    i64::MIN + rng.range(0, 3)
                } else if rng.chance(50) {
                    i64::MAX - rng.range(0, 3)
                } else {
                    i64::MIN + rng.range(0, 3)
                }
            };
            let orders = (0..n)
                .map(|i| {
                    let s = side(rng);
                    let qty = u64::MAX - rng.below(3);
                    let time = rng.below(3);
                    if rng.chance(15) {
                        Order::market(i as u64, s, qty, time)
                    } else {
                        Order::limit(i as u64, s, price(rng), qty, time)
                    }
                })
                .collect();
            let reference = match rng.below(3) {
                0 => None,
                1 => Some(i64::MAX - rng.range(0, 5)),
                _ => Some(i64::MIN + rng.range(0, 5)),
            };
            Generated { orders, reference }
        }
        Profile::Mixed => match rng.below(6) {
            0 => {
                let s = side(rng);
                let mut g = generate(Profile::Uniform, n, rng);
                for o in &mut g.orders {
                    o.side = s;
                }
                g
            }
            1 => generate(Profile::Uniform, 1, rng),
            2 => generate(Profile::Uniform, n, rng),
            3 => generate(Profile::MarketHeavy, n, rng),
            4 => generate(Profile::Extreme, n, rng),
            _ => generate(Profile::TieHeavy, n, rng),
        },
    }
}

/// A book with exactly `levels` distinct limit prices, one order per price,
/// sides and quantities random so bids and asks overlap across the range.
pub fn generate_depth(levels: usize, rng: &mut Rng) -> Vec<Order> {
    (0..levels)
        .map(|i| {
            Order::limit(
                i as u64,
                side(rng),
                i as i64,
                rng.range(1, 1_000) as u64,
                rng.below(levels as u64),
            )
        })
        .collect()
}
