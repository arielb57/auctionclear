//! One small book per tie-break step of each preset. Each test runs the preset
//! and the preset with that step removed (or, for a terminal step, swapped for
//! another terminal) and asserts the outcomes differ as stated. Deleting a
//! step from a preset makes its test fail.

mod support;

use auctionclear::{Book, Order, Outcome, Policy, Side, Step, UncrossError, Venue};

fn b(id: u64, price: i64, qty: u64, time: u64) -> Order {
    Order::limit(id, Side::Buy, price, qty, time)
}

fn s(id: u64, price: i64, qty: u64, time: u64) -> Order {
    Order::limit(id, Side::Sell, price, qty, time)
}

fn uncross(orders: &[Order], steps: Vec<Step>, reference: i64) -> Result<Outcome, UncrossError> {
    let policy = Policy::new(steps).unwrap();
    let out = Book::new(orders.to_vec())
        .unwrap()
        .uncross(&policy, Some(reference));
    assert_eq!(
        out,
        support::reference::uncross(orders, policy.steps(), Some(reference))
    );
    if let Ok(o) = &out {
        support::check_invariants(orders, o, Some(reference)).unwrap();
    }
    out
}

fn price(out: &Result<Outcome, UncrossError>) -> i64 {
    match out {
        Ok(Outcome::Cleared(c)) => c.price,
        other => panic!("expected a clearing, got {other:?}"),
    }
}

fn without(venue: Venue, step: Step) -> Vec<Step> {
    let mut steps = venue.steps();
    let at = steps
        .iter()
        .position(|&s| s == step)
        .expect("step is in the preset");
    steps.remove(at);
    steps
}

fn replaced(venue: Venue, from: Step, to: Step) -> Vec<Step> {
    venue
        .steps()
        .into_iter()
        .map(|s| if s == from { to } else { s })
        .collect()
}

#[test]
fn max_volume_beats_smaller_imbalance() {
    // At 10: demand 101, supply 50 -> volume 50, imbalance +51.
    // At 20: demand 1, supply 50 -> volume 1, imbalance -49.
    let orders = [b(1, 20, 1, 0), b(2, 10, 100, 1), s(3, 10, 50, 2)];
    let book = Book::new(orders.to_vec()).unwrap();
    let cands = book.candidates(Some(20));
    let least_imbalance = cands
        .iter()
        .min_by_key(|c| c.imbalance.unsigned_abs())
        .unwrap();
    assert_eq!((least_imbalance.price, least_imbalance.volume), (20, 1));
    for venue in Venue::ALL {
        let out = uncross(&orders, venue.steps(), 20);
        assert_eq!(price(&out), 10, "{}", venue.name());
        let Ok(Outcome::Cleared(c)) = out else {
            unreachable!()
        };
        assert_eq!((c.volume, c.fills), (50, vec![1, 49, 50]));
    }
    assert_eq!(
        Policy::new(vec![Step::MinAbsImbalance, Step::Lowest]),
        Err(auctionclear::PolicyError::MustStartWithMaxVolume)
    );
}

#[test]
fn sse_eligible() {
    // 100 and 101/102 tie on volume 3; at 102 and between, the asks at 100
    // (5 shares) cannot all fill.
    let orders = [s(0, 100, 2, 2), b(1, 102, 3, 1), s(2, 100, 3, 3)];
    let with = uncross(&orders, Venue::Sse.steps(), 97);
    let Ok(Outcome::Cleared(c)) = &with else {
        panic!()
    };
    assert_eq!((c.price, c.fills.clone()), (100, vec![2, 3, 1]));
    assert_eq!(
        uncross(&orders, without(Venue::Sse, Step::Eligible), 97),
        Err(UncrossError::Unclearable { price: 101 })
    );
}

#[test]
fn sse_min_abs_imbalance() {
    // Eligible prices 101 (imbalance 0) and 104 (imbalance -3).
    let orders = [s(0, 101, 3, 0), b(1, 106, 3, 2), s(2, 104, 3, 1)];
    assert_eq!(price(&uncross(&orders, Venue::Sse.steps(), 104)), 101);
    assert_eq!(
        price(&uncross(
            &orders,
            without(Venue::Sse, Step::MinAbsImbalance),
            104
        )),
        102
    );
}

#[test]
fn sse_midpoint() {
    let orders = [b(0, 102, 2, 2), s(1, 100, 2, 3)];
    assert_eq!(price(&uncross(&orders, Venue::Sse.steps(), 103)), 101);
    assert_eq!(
        price(&uncross(
            &orders,
            replaced(Venue::Sse, Step::Midpoint, Step::Lowest),
            103
        )),
        100
    );
}

#[test]
fn szse_eligible() {
    let orders = [s(0, 99, 2, 1), b(1, 101, 1, 0), s(2, 99, 1, 3)];
    let with = uncross(&orders, Venue::Szse.steps(), 101);
    let Ok(Outcome::Cleared(c)) = &with else {
        panic!()
    };
    assert_eq!((c.price, c.fills.clone()), (99, vec![1, 1, 0]));
    assert_eq!(
        uncross(&orders, without(Venue::Szse, Step::Eligible), 101),
        Err(UncrossError::Unclearable { price: 101 })
    );
}

#[test]
fn szse_min_abs_imbalance() {
    // 102 is nearer the reference but has imbalance +2; 104 has 0.
    let orders = [b(0, 104, 2, 0), b(1, 102, 2, 1), s(2, 100, 2, 0)];
    assert_eq!(price(&uncross(&orders, Venue::Szse.steps(), 100)), 104);
    assert_eq!(
        price(&uncross(
            &orders,
            without(Venue::Szse, Step::MinAbsImbalance),
            100
        )),
        102
    );
}

#[test]
fn szse_nearest_reference() {
    let orders = [b(0, 102, 2, 3), s(1, 101, 2, 3)];
    assert_eq!(price(&uncross(&orders, Venue::Szse.steps(), 106)), 102);
    assert_eq!(
        price(&uncross(
            &orders,
            without(Venue::Szse, Step::NearestReference),
            106
        )),
        101
    );
}

#[test]
fn szse_midpoint_of_equidistant_prices_is_the_reference() {
    let orders = [b(0, 102, 1, 0), s(1, 100, 1, 0)];
    assert_eq!(price(&uncross(&orders, Venue::Szse.steps(), 101)), 101);
    assert_eq!(
        price(&uncross(
            &orders,
            replaced(Venue::Szse, Step::Midpoint, Step::Lowest),
            101
        )),
        100
    );
}

#[test]
fn nasdaq_eligible() {
    // Rule text alone (nearest to 97, i.e. 98) would give the 103 bid only 2 of 3.
    let orders = [s(0, 98, 2, 1), b(1, 103, 3, 1)];
    assert_eq!(price(&uncross(&orders, Venue::Nasdaq.steps(), 97)), 103);
    assert_eq!(
        uncross(&orders, without(Venue::Nasdaq, Step::Eligible), 97),
        Err(UncrossError::Unclearable { price: 98 })
    );
}

#[test]
fn nasdaq_min_abs_imbalance() {
    let orders = [b(0, 102, 1, 3), b(1, 104, 1, 2), s(2, 102, 1, 1)];
    assert_eq!(price(&uncross(&orders, Venue::Nasdaq.steps(), 100)), 104);
    assert_eq!(
        price(&uncross(
            &orders,
            without(Venue::Nasdaq, Step::MinAbsImbalance),
            100
        )),
        102
    );
}

#[test]
fn nasdaq_nearest_reference() {
    let orders = [b(0, 99, 3, 0), s(1, 98, 3, 2)];
    assert_eq!(price(&uncross(&orders, Venue::Nasdaq.steps(), 103)), 99);
    assert_eq!(
        price(&uncross(
            &orders,
            without(Venue::Nasdaq, Step::NearestReference),
            103
        )),
        98
    );
}

#[test]
fn nasdaq_lowest_of_equidistant_prices() {
    let orders = [b(0, 102, 3, 3), s(1, 100, 3, 0)];
    assert_eq!(price(&uncross(&orders, Venue::Nasdaq.steps(), 101)), 100);
    assert_eq!(
        price(&uncross(
            &orders,
            replaced(Venue::Nasdaq, Step::Lowest, Step::Highest),
            101
        )),
        102
    );
}

#[test]
fn xetra_min_abs_imbalance() {
    let orders = [b(0, 100, 2, 3), b(1, 101, 3, 3), s(2, 100, 3, 0)];
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 99)), 101);
    assert_eq!(
        price(&uncross(
            &orders,
            without(Venue::Xetra, Step::MinAbsImbalance),
            99
        )),
        100
    );
}

#[test]
fn xetra_imbalance_side() {
    // 10 and 11 both execute 3 with a buy surplus of 2: the highest wins.
    let orders = [b(0, 11, 5, 0), s(1, 10, 3, 1)];
    let rule_text = vec![
        Step::MaxVolume,
        Step::MinAbsImbalance,
        Step::ImbalanceSide,
        Step::ClampReference,
    ];
    assert_eq!(price(&uncross(&orders, rule_text, 10)), 11);
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 10)), 11);
    let no_side = vec![Step::MaxVolume, Step::MinAbsImbalance, Step::ClampReference];
    assert_eq!(
        uncross(&orders, no_side, 10),
        Err(UncrossError::Unclearable { price: 10 })
    );

    // Sell surplus at both 10 and 11: the lowest wins.
    let orders = [b(0, 11, 3, 0), s(1, 10, 5, 1)];
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 11)), 10);
}

/// In the Xetra preset, Eligible makes ImbalanceSide redundant: when every
/// tied price has buy pressure, only the highest lets every bid above it fill.
/// This test records that; if a change to either step broke the equivalence
/// it would fail.
#[test]
fn xetra_imbalance_side_is_implied_by_eligible() {
    use auctionclear::gen::{generate, Profile, Rng};
    let mut rng = Rng::new(42);
    for _ in 0..50_000 {
        let n = 1 + rng.below(10) as usize;
        let g = generate(Profile::TieHeavy, n, &mut rng);
        let book = Book::new(g.orders).unwrap();
        let full = book.uncross(&Venue::Xetra.policy(), g.reference);
        let reduced = book.uncross(
            &Policy::new(without(Venue::Xetra, Step::ImbalanceSide)).unwrap(),
            g.reference,
        );
        assert_eq!(full, reduced);
    }
}

#[test]
fn xetra_eligible() {
    // Market-free version of the crowding case: at 99 the bid at 104 and the
    // bid at 100 compete for one share.
    let orders = [
        b(0, 100, 1, 2),
        s(1, 99, 1, 2),
        b(2, 104, 1, 0),
        s(3, 103, 1, 2),
    ];
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 99)), 100);
    assert_eq!(
        uncross(&orders, without(Venue::Xetra, Step::Eligible), 99),
        Err(UncrossError::Unclearable { price: 99 })
    );

    // Market bids 25 against market asks 19: without Eligible the reference
    // price 50 is chosen and the bid at 51 gets nothing.
    let mut orders = vec![s(0, 50, 6, 10), b(1, 51, 5, 4), s(2, 52, 5, 12)];
    for (i, q) in [7u64, 4, 3, 9, 2].iter().enumerate() {
        orders.push(Order::market(10 + i as u64, Side::Buy, *q, i as u64));
    }
    for (i, q) in [1u64, 8, 3, 7].iter().enumerate() {
        orders.push(Order::market(20 + i as u64, Side::Sell, *q, i as u64));
    }
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 50)), 51);
    assert_eq!(
        uncross(&orders, without(Venue::Xetra, Step::Eligible), 50),
        Err(UncrossError::Unclearable { price: 50 })
    );
}

#[test]
fn xetra_clamp_reference() {
    // Reference inside the tied range: the reference price itself.
    let orders = [b(0, 104, 1, 0), s(1, 100, 1, 0)];
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 101)), 101);
    assert_eq!(
        price(&uncross(
            &orders,
            replaced(Venue::Xetra, Step::ClampReference, Step::Midpoint),
            101
        )),
        102
    );
    // Reference above the range: the highest tied price.
    let orders = [b(0, 100, 2, 3), s(1, 99, 2, 1)];
    assert_eq!(price(&uncross(&orders, Venue::Xetra.steps(), 104)), 100);
    assert_eq!(
        price(&uncross(
            &orders,
            replaced(Venue::Xetra, Step::ClampReference, Step::Lowest),
            104
        )),
        99
    );
}
