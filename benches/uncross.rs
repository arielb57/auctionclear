//! Uncross time against book depth, sweep versus brute-force reference.
//!
//! Run with `cargo bench --bench uncross`. Each book has one order per
//! distinct price level. Times are the median of several runs and include
//! building the book (sorting and aggregating levels).

#[path = "../tests/support/reference.rs"]
mod reference;

use std::hint::black_box;
use std::time::{Duration, Instant};

use auctionclear::gen::{generate_depth, Rng};
use auctionclear::{Book, Venue};

fn median_time(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

fn fmt(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us < 1_000.0 {
        format!("{us:.1} µs")
    } else if us < 1_000_000.0 {
        format!("{:.2} ms", us / 1_000.0)
    } else {
        format!("{:.2} s", us / 1_000_000.0)
    }
}

fn main() {
    let policy = Venue::Xetra.policy();
    let steps = policy.steps().to_vec();
    println!("| levels (= orders) | sweep | sweep ns/level | brute force | brute / sweep |");
    println!("|---:|---:|---:|---:|---:|");
    for &levels in &[10usize, 100, 1_000, 10_000, 100_000, 1_000_000] {
        let orders = generate_depth(levels, &mut Rng::new(levels as u64));
        let reference = Some(levels as i64 / 2);
        let runs = if levels >= 100_000 { 5 } else { 21 };
        let sweep = median_time(runs, || {
            let book = Book::new(black_box(orders.clone())).unwrap();
            black_box(book.uncross(&policy, reference).unwrap());
        });
        let per_level = sweep.as_secs_f64() * 1e9 / levels as f64;
        if levels <= 10_000 {
            let brute_runs = if levels >= 10_000 { 3 } else { 11 };
            let brute = median_time(brute_runs, || {
                black_box(reference::uncross(black_box(&orders), &steps, reference).unwrap());
            });
            println!(
                "| {levels} | {} | {per_level:.0} | {} | {:.0}x |",
                fmt(sweep),
                fmt(brute),
                brute.as_secs_f64() / sweep.as_secs_f64()
            );
        } else {
            println!(
                "| {levels} | {} | {per_level:.0} | not run (quadratic) | |",
                fmt(sweep)
            );
        }
    }
}
