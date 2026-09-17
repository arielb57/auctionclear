mod support;

use auctionclear::{
    csv, Book, BookError, NoTradeReason, Order, Outcome, Policy, PolicyError, Side, Step,
    UncrossError, Venue,
};

fn all_venues(orders: &[Order], reference: Option<i64>) -> Outcome {
    let book = Book::new(orders.to_vec()).unwrap();
    let mut first = None;
    for v in Venue::ALL {
        let out = book.uncross(&v.policy(), reference).unwrap();
        assert_eq!(
            Ok(out.clone()),
            support::reference::uncross(orders, &v.steps(), reference)
        );
        support::check_invariants(orders, &out, reference).unwrap();
        first.get_or_insert(out);
    }
    first.unwrap()
}

fn cleared(out: Outcome) -> auctionclear::Clearing {
    match out {
        Outcome::Cleared(c) => c,
        other => panic!("expected a clearing, got {other:?}"),
    }
}

#[test]
fn empty_book_and_empty_side_do_not_trade() {
    assert_eq!(
        all_venues(&[], Some(100)),
        Outcome::NoTrade(NoTradeReason::NoCross)
    );
    let bids_only = [
        Order::limit(1, Side::Buy, 100, 5, 0),
        Order::market(2, Side::Buy, 5, 1),
    ];
    assert_eq!(
        all_venues(&bids_only, Some(100)),
        Outcome::NoTrade(NoTradeReason::NoCross)
    );
    let asks_only = [Order::market(1, Side::Sell, 5, 0)];
    assert_eq!(
        all_venues(&asks_only, None),
        Outcome::NoTrade(NoTradeReason::NoCross)
    );
}

#[test]
fn no_overlap_does_not_trade() {
    let orders = [
        Order::limit(1, Side::Buy, 99, 5, 0),
        Order::limit(2, Side::Sell, 100, 5, 1),
    ];
    assert_eq!(
        all_venues(&orders, Some(100)),
        Outcome::NoTrade(NoTradeReason::NoCross)
    );
}

#[test]
fn only_market_orders_clear_at_the_reference_price() {
    let orders = [
        Order::market(1, Side::Buy, 30, 5),
        Order::market(2, Side::Buy, 30, 1),
        Order::market(3, Side::Sell, 40, 2),
    ];
    let c = cleared(all_venues(&orders, Some(2_500)));
    assert_eq!((c.price, c.volume, c.imbalance), (2_500, 40, 20));
    // Time priority: order 2 (time 1) fills first.
    assert_eq!(c.fills, vec![10, 30, 40]);

    assert_eq!(
        all_venues(&orders, None),
        Outcome::NoTrade(NoTradeReason::NoReferencePrice)
    );
}

#[test]
fn single_level() {
    let orders = [
        Order::limit(1, Side::Buy, 100, 5, 0),
        Order::limit(2, Side::Sell, 100, 3, 1),
    ];
    let c = cleared(all_venues(&orders, None));
    assert_eq!(
        (c.price, c.volume, c.imbalance, c.fills),
        (100, 3, 2, vec![3, 3])
    );
}

#[test]
fn at_price_side_fills_in_time_order_with_one_partial() {
    let orders = [
        Order::limit(1, Side::Sell, 100, 10, 0),
        Order::limit(2, Side::Buy, 100, 4, 5),
        Order::limit(3, Side::Buy, 100, 4, 1),
        Order::limit(4, Side::Buy, 100, 4, 3),
        Order::limit(5, Side::Buy, 100, 4, 3),
    ];
    let c = cleared(all_venues(&orders, Some(100)));
    // Times 1, 3, 3 (position breaks the tie), 5.
    assert_eq!(c.fills, vec![10, 0, 4, 4, 2]);
}

#[test]
fn market_orders_fill_before_limit_orders() {
    let orders = [
        Order::limit(1, Side::Buy, 105, 10, 0),
        Order::market(2, Side::Buy, 6, 9),
        Order::market(3, Side::Buy, 6, 8),
        Order::limit(4, Side::Sell, 105, 8, 0),
    ];
    let c = cleared(all_venues(&orders, Some(105)));
    assert_eq!((c.price, c.volume), (105, 8));
    assert_eq!(c.fills, vec![0, 2, 6, 8]);
}

#[test]
fn quantities_near_u64_max_do_not_overflow() {
    let big = u64::MAX;
    let orders = [
        Order::limit(1, Side::Sell, 100, big, 0),
        Order::limit(2, Side::Sell, 100, big, 1),
        Order::market(3, Side::Buy, big, 2),
        Order::limit(4, Side::Buy, 101, big - 1, 3),
    ];
    let c = cleared(all_venues(&orders, Some(100)));
    assert_eq!(c.volume, 2 * big as u128 - 1);
    // Sell side is short one share; the later sell takes the partial fill.
    assert_eq!(c.fills, vec![big, big - 1, big, big - 1]);
    assert_eq!(c.imbalance, -1);
}

#[test]
fn prices_at_i64_limits() {
    let orders = [
        Order::limit(1, Side::Buy, i64::MAX, 1, 0),
        Order::limit(2, Side::Sell, i64::MIN, 1, 0),
    ];
    let book = Book::new(orders.to_vec()).unwrap();
    // Two prices tie on everything; floor((MIN + MAX) / 2) = -1.
    let sse = book.uncross(&Venue::Sse.policy(), None).unwrap();
    assert_eq!(cleared(sse.clone()).price, -1);
    assert_eq!(
        Ok(sse),
        support::reference::uncross(&orders, &Venue::Sse.steps(), None)
    );
    let xetra = book
        .uncross(&Venue::Xetra.policy(), Some(i64::MIN))
        .unwrap();
    assert_eq!(cleared(xetra).price, i64::MIN);
    let nasdaq = book
        .uncross(&Venue::Nasdaq.policy(), Some(i64::MAX))
        .unwrap();
    assert_eq!(cleared(nasdaq).price, i64::MAX);
}

#[test]
fn zero_quantity_is_rejected() {
    let err = Book::new(vec![Order::limit(7, Side::Buy, 1, 0, 0)]).unwrap_err();
    assert_eq!(err, BookError::ZeroQuantity { id: 7 });
}

#[test]
fn invalid_chains_are_rejected() {
    use Step::*;
    assert_eq!(
        Policy::new(vec![]),
        Err(PolicyError::MustStartWithMaxVolume)
    );
    assert_eq!(
        Policy::new(vec![MaxVolume, MinAbsImbalance]),
        Err(PolicyError::MustEndWithTerminal)
    );
    assert_eq!(
        Policy::new(vec![MaxVolume, Lowest, Highest]),
        Err(PolicyError::TerminalBeforeEnd(Lowest))
    );
    assert_eq!(
        Policy::new(vec![MaxVolume, MaxVolume, Lowest]),
        Err(PolicyError::RepeatedMaxVolume)
    );
    assert!(Policy::new(vec![MaxVolume, Midpoint]).is_ok());
}

#[test]
fn custom_chain_can_pick_an_unclearable_price() {
    // 10 and 12 both execute 5; at 12 the 7-share ask at 10 is only partly filled.
    let orders = [
        Order::limit(1, Side::Buy, 12, 5, 0),
        Order::limit(2, Side::Sell, 10, 7, 0),
    ];
    let book = Book::new(orders.to_vec()).unwrap();
    let highest = Policy::new(vec![Step::MaxVolume, Step::Highest]).unwrap();
    assert_eq!(
        book.uncross(&highest, None),
        Err(UncrossError::Unclearable { price: 12 })
    );
    let lowest = Policy::new(vec![Step::MaxVolume, Step::Lowest]).unwrap();
    assert_eq!(cleared(book.uncross(&lowest, None).unwrap()).price, 10);
}

#[test]
fn csv_round_trip_and_errors() {
    let orders = vec![
        Order::limit(1, Side::Buy, -5, 10, 3),
        Order::market(2, Side::Sell, u64::MAX, 0),
    ];
    let text = csv::write(&orders, Some(-4));
    let parsed = csv::parse(&text).unwrap();
    assert_eq!((parsed.orders, parsed.reference), (orders, Some(-4)));

    let parsed = csv::parse("# a comment\n\n1, B, MKT, 3, 0\n2,s,7,1,1\n").unwrap();
    assert_eq!(parsed.orders[0], Order::market(1, Side::Buy, 3, 0));
    assert_eq!(parsed.reference, None);

    let err = csv::parse("id,side,price,qty,time\n1,buy,10,0,0\n").unwrap_err();
    assert_eq!(err.line, 2);
    assert!(csv::parse("1,hold,10,1,0")
        .unwrap_err()
        .message
        .contains("side"));
    assert!(csv::parse("1,buy,ten,1,0")
        .unwrap_err()
        .message
        .contains("price"));
    assert!(csv::parse("1,buy,10,1")
        .unwrap_err()
        .message
        .contains("5 fields"));
    assert!(csv::parse("# reference: abc")
        .unwrap_err()
        .message
        .contains("reference"));
}
