# auctionclear

Call-auction uncrossing in Rust with venue-style tie-break chains, checked against a brute-force reference.

## The problem

Opening and closing auctions set one price: the one that executes the most volume.
Often several prices execute the same volume, and venues break that tie in
different ways: smallest imbalance, the side the imbalance is on, distance to a
reference price, a midpoint. Simulators and research code usually take the first
price that maximises volume, so their auction prints and fills don't match the
venue's. If you simulate auctions or rebuild auction prints from order data,
you have no reference implementation to check yours against. This crate is
meant to be one. It is small, has no dependencies, and every result is checked
against an independent brute-force implementation.

## How it works

**Aggregation and sweep.** Limit orders are sorted into distinct price levels
(O(n log n)). Market buys count as bids at +∞ and market sells as asks at −∞.
Two cumulative arrays, bids at or above each level and asks at or below it,
give every candidate price in one O(levels) pass:

- `demand(p)` = market bids + limit bids ≥ p
- `supply(p)` = market asks + limit asks ≤ p
- `volume(p) = min(demand, supply)`, `imbalance(p) = demand − supply` (signed)
- `eligible(p)`: every limit order priced *through* p (a bid above it or an ask
  below it) can fill completely, taking into account that market orders fill first.

Quantities are `u64` shares, but the aggregates are `u128` and the imbalance is
`i128`, so books full of `u64::MAX` orders don't overflow. Prices are `i64` ticks.

**Policy chain.** A price is chosen by a chain of steps. Each filter narrows the
candidate set, and exactly one terminal step at the end produces the price:

| step | kind | keeps / returns |
|---|---|---|
| `MaxVolume` | filter (always first) | largest executable volume |
| `Eligible` | filter | prices where every order priced through fills completely |
| `MinAbsImbalance` | filter | smallest \|demand − supply\| |
| `ImbalanceSide` | filter | the highest price if all have buy pressure, the lowest if all have sell pressure |
| `NearestReference` | filter | closest to the reference price |
| `Midpoint` | terminal | `floor((lowest + highest) / 2)` |
| `ClampReference` | terminal | the reference price clamped into `[lowest, highest]` |
| `Lowest` / `Highest` | terminal | an end of the remaining range |

**Presets.** Each preset paraphrases a published rule. `auctionclear venues`
prints the paraphrase and names the steps that are interpretation rather than
rule text.

| venue | chain | interpretation |
|---|---|---|
| `sse` | MaxVolume → Eligible → MinAbsImbalance → Midpoint | "least unexecuted volume" read as \|imbalance\|; midpoint rounds down |
| `szse` | MaxVolume → Eligible → MinAbsImbalance → NearestReference → Midpoint | Midpoint for two prices equidistant from the reference |
| `nasdaq` | MaxVolume → Eligible → MinAbsImbalance → NearestReference → Lowest | Eligible and Lowest are not in Rule 4752/4754 text |
| `xetra` | MaxVolume → MinAbsImbalance → ImbalanceSide → Eligible → ClampReference | Eligible is not in the rule text; midpoint when there is no reference |

**Price collars.** Everything above answers "which price?". A venue asks a prior
question — *should this print?* If the price the book would clear at sits too
far from a reference, no major exchange simply prints it: Xetra calls the result
a volatility interruption, Euronext a reservation, the US markets an extension
under the limit up-limit down bands. In each case the auction is prolonged and
the imbalance published so somebody can react to it.

That makes a collar a third outcome rather than another filter. Dropping the
offending price from the candidate set would be wrong twice over: the auction
would print at some other, worse price, and the caller would never learn the
venue had stepped in.

```
$ cat spike.csv
# reference: 100
id,side,price,qty,time
1,buy,140,500,0
2,sell,135,500,1

$ auctionclear run spike.csv --venue sse
price      137

$ auctionclear run spike.csv --venue sse --collar-bps 1000
price      none (auction extended)
collar     137 is 3700 bps from the reference 100, past the 1000 bps collar
indicative 137
volume     500 (would have traded)
imbalance  0
```

The four presets disagree about the price. They cannot disagree about whether
the venue steps in, because each checks the price it chose — `tests/collar.rs`
pins that, along with the band edge printing (rounding is the permissive
direction), a missing reference meaning no band rather than no trade, and a
book that never crossed staying a no-trade instead of becoming an extension.

**Allocation.** At the chosen price the executable orders on each side fall into
three priority groups: market orders, limit orders priced through the price, and
limit orders at the price. Groups fill whole, in that order, until one doesn't
fit. Only that group is sorted (by price, then time, then position in the file)
and rationed, so there is at most one partial fill per side. On an eligible price,
the through-priced group always fits.

**Worked example** (`examples/tie.csv`: buy 500 @ 104, sell 500 @ 100, reference 101).
Both 100 and 104 execute 500 with zero imbalance, and the four presets
produce three different prices: `sse` takes the midpoint 102, `szse` and `nasdaq` take 100
(nearest the reference), `xetra` takes the reference price 101 itself.

## Install and usage

Requires a Rust toolchain (stable). There are no runtime dependencies.

```
git clone <this repository> && cd auctionclear
cargo build --release
```

Order files are CSV: `id,side,price,qty,time`. `side` is `buy`/`sell`, `price` is
an integer tick or `market`, and lower `time` means higher priority. A
`# reference: <ticks>` comment sets the reference price; `--reference` overrides it.

```
$ cargo run --release -q -- run examples/opening.csv --venue sse --levels
venue      sse (MaxVolume -> Eligible -> MinAbsImbalance -> Midpoint)
reference  100

candidates
       price       demand       supply       volume    imbalance  eligible
          99          700          250          250          450  no
         100          700          400          400          300  no
         101          600          400          400          200  yes
         102          300          600          300         -300  no
         103          300          700          300         -400  no

price      101
volume     400
imbalance  +200 (buy surplus)

      id  side        limit        qty     time     filled
       1   buy       market        100        0        100
       2   buy          103        200        1        200
       3   buy          101        300        2        100
       4  sell           99        250        3        250
       5  sell          100        150        4        150
       6  sell          102        200        5          0
       7   buy          100        100        6          0
       8  sell          103        100        7          0
```

Here 100 and 101 tie on volume 400. At 100 the market buy and the bids at 101 and 103
add up to 600 shares priced through the auction price, and only 400 can trade,
so 100 is not eligible.

Generate random books, including tie-heavy ones, and list the presets:

```
$ cargo run --release -q -- gen --profile tie-heavy --orders 6 --seed 7
# reference: 104
id,side,price,qty,time
0,buy,99,3,0
1,buy,101,1,1
2,sell,100,1,3
3,sell,101,3,3
4,sell,99,2,1
5,sell,99,2,1

$ cargo run --release -q -- venues
```

Profiles: `uniform`, `tie-heavy`, `market-heavy`, `extreme` (quantities near
`u64::MAX`, prices at the ends of `i64`), `mixed`.

As a library:

```rust
use auctionclear::{Book, Order, Outcome, Side, Venue};

let book = Book::new(vec![
    Order::limit(1, Side::Buy, 101, 300, 0),
    Order::limit(2, Side::Buy, 100, 200, 1),
    Order::limit(3, Side::Sell, 99, 400, 2),
    Order::market(4, Side::Sell, 50, 3),
]).unwrap();
if let Outcome::Cleared(c) = book.uncross(&Venue::Xetra.policy(), Some(100)).unwrap() {
    assert_eq!((c.price, c.volume, c.fills), (100, 450, vec![300, 150, 400, 50]));
}
```

Custom chains are built with `Policy::new(vec![Step::MaxVolume, ..., Step::Lowest])`.

## Tests

```
cargo test
```

The suite runs in a few seconds (the test profile is optimised).

- **Differential** (`tests/differential.rs`): 105,000 seeded random books across
  five profiles. Each book runs under all four presets, and the result must equal
  the brute-force reference (`tests/support/reference.rs`) on the outcome, price,
  volume, imbalance and every per-order fill. In the tie-heavy profile more than
  15,000 of the 40,000 books tie on maximum volume (the test asserts that). Another
  20,000 books run under random custom chains, and there the sweep and the reference
  must also agree on errors.
  The reference shares none of the sweep code: it rescans every order at every
  candidate price, decides eligibility by simulating the allocation, and fills
  by fully sorting each side.
- **Invariants** on every one of those outcomes: bought = sold = reported volume.
  No buy fills above the price and no sell fills below it. Every limit order priced
  through the price fills completely. Market orders fill before limits. The at-price
  group fills in time order with at most one partial fill, and at most one side is short.
  The residual book is not crossed. No candidate price executes more volume.
- **Worked examples** (`tests/worked_examples.rs`): one book per step per preset.
  Each test runs the preset with the step removed (or a terminal swapped) and
  asserts that the result changes.
- **Edge cases** (`tests/edge_cases.rs`): empty book, one empty side, no overlap,
  only market orders (reference price decides; none → no trade), single level,
  quantities at `u64::MAX`, prices at `i64::MIN`/`MAX`, invalid chains, CSV errors.
- **CLI** (`tests/cli.rs`): runs the binary end to end.

Failures print the offending book as CSV, so you can replay it with `auctionclear run`.
There is no shrinking, so a failure can't hang.

## Results

```
cargo bench --bench uncross
```

Each book has one order per distinct price level. Sides are random, so bids and
asks overlap across the whole range. Each time is the median of 21 runs (5 for the
two largest books, 3 for the 10,000-level brute force). It covers building the book
(sorting orders into levels) plus the uncross under the `xetra` preset. Measured on
an Apple Silicon (arm64) Mac, macOS, Rust 1.94.1, single thread:

| levels (= orders) | sweep | sweep ns/level | brute force | brute / sweep |
|---:|---:|---:|---:|---:|
| 10 | 2.4 µs | 242 | 9.0 µs | 4x |
| 100 | 6.8 µs | 68 | 322.3 µs | 48x |
| 1,000 | 53.6 µs | 54 | 7.90 ms | 147x |
| 10,000 | 202.9 µs | 20 | 1.12 s | 5509x |
| 100,000 | 2.56 ms | 26 | not run | |
| 1,000,000 | 29.21 ms | 29 | not run | |

Per-level cost stays flat at 20–30 ns from 10,000 to 1,000,000 levels. Below that,
fixed allocation overhead dominates. A book of one million orders on one million
levels clears in about 30 ms. The brute force costs O(levels × orders · log orders),
because it simulates an allocation at every candidate, and at 10,000 levels it is
already over a second. It exists to check correctness, not to be fast. Expect your
numbers to differ with your hardware.

## Design notes

**Eligibility is its own step, not an assumption.** At first I expected
MinAbsImbalance plus ImbalanceSide to guarantee that orders priced through the
auction price always fill completely, since that holds in the simple cases. The
differential run found a counterexample within seconds: market bids of 25 against
market asks of 19, with tied prices 50, 51 and 52. The reference price 50 is chosen,
and the bid at 51 gets nothing because the market orders absorb all the volume.
`Eligible` is the SSE/SZSE conditions (2) and (3) turned into a filter. It is written
into the SSE and SZSE chains because their rule text states it, and added to the Nasdaq
and Xetra chains as a labelled interpretation. A custom chain can leave it out. If
that chain then picks a price where an order priced through the auction price can't
fill completely, `uncross` returns `UncrossError::Unclearable` instead of quietly
rationing. The error path is tested against the reference too.

A side effect is that `ImbalanceSide` never changes the Xetra preset's result
once `Eligible` follows it. If every tied price has buy pressure, the highest is the
only one at which every bid above it fills. That is ImbalanceSide's answer too. The
step stays because it is the rule text. `xetra_imbalance_side` shows it deciding the
price on the rule-text chain, and `xetra_imbalance_side_is_implied_by_eligible`
records the equivalence over 50,000 books, so it fails if either step's meaning changes.

**Allocation rations one group, not the book.** The obvious allocator sorts each
side by priority and fills greedily. That is what the reference does. The sweep
instead uses the level prefix sums to find which priority group is cut. It fills
the others in a single pass and sorts only the cut group. Most books need no sort
at all, and the two allocators are different enough that agreement between them
means something.

## Limitations

- The presets are "style" presets, paraphrased from rule text. They are not
  certified reproductions of any exchange. Real auctions add things this crate
  doesn't model: hidden or iceberg quantity, odd-lot and minimum-quantity
  handling, and the venue's exact choice of reference price.
- The collar is one static symmetric band. Real books layer a dynamic band
  against the last trade on top of a static one against the previous close, and
  the widths vary by instrument, by time of day and by how many extensions have
  already happened. `--collar-bps` models the shape, not any venue's table, and
  nothing here decides how long an extension lasts or what happens at the end
  of it.
- Candidate prices are the limit prices in the book. The reference price is a
  candidate only when there are no limit orders. Midpoint and ClampReference
  can return a price between levels, and the tests check it is clearable there.
- Rounding: Midpoint rounds down to a whole tick. Tick sizes are not modelled,
  since prices are already integer ticks.
- A book with only market orders and no reference price does not trade
  (`NoReferencePrice`), and neither does a book with no crossing volume. Neither
  reports an indicative price or imbalance.
- The brute-force reference lives under `tests/` and is not part of the published
  crate API.

## License

MIT. See [LICENSE](LICENSE).
