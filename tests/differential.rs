//! Sweep versus brute force on random books, plus book invariants.
//!
//! Books are generated from a fixed seed so any failure is reproducible; the
//! failing book is printed as CSV. No shrinking is involved.

mod support;

use auctionclear::gen::{generate, Profile, Rng};
use auctionclear::{csv, Book, Outcome, Policy, Step, Venue};

fn run_profile(profile: Profile, books: usize, max_orders: u64, seed: u64) -> (usize, usize) {
    let mut rng = Rng::new(seed);
    let policies: Vec<(String, Policy)> = Venue::ALL
        .iter()
        .map(|v| (v.name().to_string(), v.policy()))
        .collect();
    let mut cleared = 0;
    let mut tied = 0;
    for book_no in 0..books {
        let n = 1 + rng.below(max_orders) as usize;
        let g = generate(profile, n, &mut rng);
        let book = Book::new(g.orders.clone()).expect("generated quantities are positive");
        let candidates = book.candidates(g.reference);
        let best = candidates.iter().map(|c| c.volume).max().unwrap_or(0);
        if best > 0 && candidates.iter().filter(|c| c.volume == best).count() > 1 {
            tied += 1;
        }
        for (name, policy) in &policies {
            let fast = book.uncross(policy, g.reference);
            let slow = support::reference::uncross(&g.orders, policy.steps(), g.reference);
            let context = || {
                format!(
                    "profile {} book #{book_no} venue {name}\n{}",
                    profile.name(),
                    csv::write(&g.orders, g.reference)
                )
            };
            assert_eq!(fast, slow, "sweep and brute force disagree: {}", context());
            let outcome = fast.unwrap_or_else(|e| panic!("preset failed with {e}: {}", context()));
            if let Err(msg) = support::check_invariants(&g.orders, &outcome, g.reference) {
                panic!("invariant violated: {msg}\n{}", context());
            }
            if matches!(outcome, Outcome::Cleared(_)) {
                cleared += 1;
            }
        }
    }
    (cleared, tied)
}

#[test]
fn tie_heavy_books_match_brute_force() {
    let (cleared, tied) = run_profile(Profile::TieHeavy, 40_000, 16, 0x7131);
    assert!(
        cleared > 100_000,
        "only {cleared} clearings; the generator is not producing crossing books"
    );
    assert!(
        tied > 15_000,
        "only {tied} of 40000 books tie on volume; the generator lost its ties"
    );
}

#[test]
fn uniform_books_match_brute_force() {
    let (cleared, _) = run_profile(Profile::Uniform, 25_000, 40, 0xA11CE);
    assert!(cleared > 50_000);
}

#[test]
fn market_heavy_books_match_brute_force() {
    let (cleared, tied) = run_profile(Profile::MarketHeavy, 20_000, 14, 0x3A7);
    assert!(cleared > 40_000);
    assert!(tied > 2_000);
}

#[test]
fn extreme_books_match_brute_force() {
    let (cleared, _) = run_profile(Profile::Extreme, 10_000, 8, 0xE7);
    assert!(cleared > 10_000);
}

#[test]
fn mixed_books_match_brute_force() {
    run_profile(Profile::Mixed, 10_000, 20, 0x5EED);
}

/// Custom chains, including ones that pick unclearable prices, must agree
/// with the reference on the price or on the error.
#[test]
fn random_custom_chains_match_brute_force() {
    use Step::*;
    let middle = [Eligible, MinAbsImbalance, ImbalanceSide, NearestReference];
    let terminals = [Midpoint, ClampReference, Lowest, Highest];
    let mut rng = Rng::new(0xC0FFEE);
    let mut errors = 0;
    for book_no in 0..20_000 {
        let mut steps = vec![MaxVolume];
        for _ in 0..rng.below(4) {
            steps.push(middle[rng.below(middle.len() as u64) as usize]);
        }
        steps.push(terminals[rng.below(4) as usize]);
        let policy = Policy::new(steps).unwrap();
        let n = 1 + rng.below(12) as usize;
        let g = generate(Profile::TieHeavy, n, &mut rng);
        let fast = Book::new(g.orders.clone())
            .unwrap()
            .uncross(&policy, g.reference);
        let slow = support::reference::uncross(&g.orders, policy.steps(), g.reference);
        assert_eq!(
            fast,
            slow,
            "book #{book_no} chain {policy}\n{}",
            csv::write(&g.orders, g.reference)
        );
        match fast {
            Ok(outcome) => support::check_invariants(&g.orders, &outcome, g.reference)
                .unwrap_or_else(|m| panic!("{m} under {policy}")),
            Err(_) => errors += 1,
        }
    }
    // Chains such as MaxVolume -> Highest do choose unclearable prices; if none
    // ever did, this test would not be exercising the error path.
    assert!(errors > 100, "only {errors} unclearable choices");
}
