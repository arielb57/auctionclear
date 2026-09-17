//! Price collars: the auction the venue refuses to print.

use auctionclear::{Book, Collar, NoTradeReason, Order, Outcome, Side, Venue};

fn limit(id: u64, side: Side, price: i64, qty: u64, time: u64) -> Order {
    Order::limit(id, side, price, qty, time)
}

/// A book that clears far above its reference: 500 at 137 against a 100 close.
fn spike() -> Book {
    Book::new(vec![
        limit(1, Side::Buy, 140, 500, 0),
        limit(2, Side::Sell, 135, 500, 1),
    ])
    .unwrap()
}

#[test]
fn a_price_outside_the_band_extends_the_auction_instead_of_printing() {
    let policy = Venue::Sse.policy();
    let book = spike();

    // Uncollared, the auction prints at the midpoint of the tied range.
    let printed = book.uncross(&policy, Some(100)).unwrap();
    let Outcome::Cleared(c) = printed else {
        panic!("expected a print, got {printed:?}");
    };
    assert_eq!(c.price, 137);

    // A 10% collar is 3,700 bps short of that.
    let held = book
        .uncross_with_collar(&policy, Some(100), Some(Collar::new(1_000)))
        .unwrap();
    let Outcome::Extended {
        breach,
        volume,
        imbalance,
    } = held
    else {
        panic!("expected an extension, got {held:?}");
    };
    assert_eq!(breach.indicative, 137);
    assert_eq!(breach.reference, 100);
    assert_eq!(breach.deviation_bps, 3_700);
    assert_eq!(breach.limit_bps, 1_000);
    // The indicative figures still stand: this is what an imbalance feed
    // publishes while the auction runs on.
    assert_eq!(volume, c.volume);
    assert_eq!(imbalance, c.imbalance);
}

#[test]
fn a_wide_enough_collar_lets_the_same_book_print() {
    let out = spike()
        .uncross_with_collar(&Venue::Sse.policy(), Some(100), Some(Collar::new(4_000)))
        .unwrap();
    assert!(matches!(out, Outcome::Cleared(c) if c.price == 137));
}

#[test]
fn the_band_edge_prints() {
    // Rounding is the permissive direction: exactly on the band clears. 110
    // against 100 is 1,000 bps.
    let book = Book::new(vec![
        limit(1, Side::Buy, 110, 100, 0),
        limit(2, Side::Sell, 110, 100, 1),
    ])
    .unwrap();
    let at = book
        .uncross_with_collar(&Venue::Sse.policy(), Some(100), Some(Collar::new(1_000)))
        .unwrap();
    assert!(matches!(at, Outcome::Cleared(_)), "on the band: {at:?}");

    let past = book
        .uncross_with_collar(&Venue::Sse.policy(), Some(100), Some(Collar::new(999)))
        .unwrap();
    assert!(
        matches!(past, Outcome::Extended { .. }),
        "past it: {past:?}"
    );
}

#[test]
fn without_a_reference_there_is_no_band() {
    // A venue with no previous close does not invent one, so the auction
    // prints. Sse needs a reference only for the tie-break, not the collar.
    let book = Book::new(vec![
        limit(1, Side::Buy, 900, 100, 0),
        limit(2, Side::Sell, 900, 100, 1),
    ])
    .unwrap();
    let out = book
        .uncross_with_collar(&Venue::Sse.policy(), None, Some(Collar::new(1)))
        .unwrap();
    assert!(matches!(out, Outcome::Cleared(_)), "{out:?}");
}

#[test]
fn a_collar_never_turns_a_no_trade_into_an_extension() {
    // Nothing crosses, so there is no indicative price to reserve.
    let book = Book::new(vec![
        limit(1, Side::Buy, 90, 100, 0),
        limit(2, Side::Sell, 110, 100, 1),
    ])
    .unwrap();
    let out = book
        .uncross_with_collar(&Venue::Sse.policy(), Some(100), Some(Collar::new(1)))
        .unwrap();
    assert_eq!(out, Outcome::NoTrade(NoTradeReason::NoCross));
}

#[test]
fn every_venue_honours_the_collar_and_agrees_on_the_breach() {
    // The presets disagree about the price; they cannot disagree about whether
    // the venue steps in, because each one checks its own chosen price.
    for venue in Venue::ALL {
        let out = spike()
            .uncross_with_collar(&venue.policy(), Some(100), Some(Collar::new(1_000)))
            .unwrap();
        match out {
            Outcome::Extended { breach, .. } => {
                assert!(breach.deviation_bps > 1_000, "{venue:?}: {breach}");
            }
            other => panic!("{venue:?} printed through the collar: {other:?}"),
        }
    }
}

#[test]
fn deviation_is_measured_in_basis_points_both_ways() {
    assert_eq!(Collar::deviation_bps(110, 100), Some(1_000));
    assert_eq!(Collar::deviation_bps(90, 100), Some(1_000));
    assert_eq!(Collar::deviation_bps(100, 100), Some(0));
    // A reference of zero or less gives no band rather than a division by zero.
    assert_eq!(Collar::deviation_bps(100, 0), None);
    // Large prices must not overflow on the way to basis points.
    assert_eq!(Collar::deviation_bps(i64::MAX, 1), Some(u64::MAX));
}
