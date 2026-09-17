use std::process::ExitCode;

use auctionclear::gen::{self, Profile, Rng};
use auctionclear::{csv, Book, Collar, Limit, NoTradeReason, Outcome, Side, Venue};

const USAGE: &str = "\
usage:
  auctionclear run <orders.csv> --venue <sse|szse|nasdaq|xetra> [--reference <ticks>]
                                [--collar-bps <n>] [--levels]
  auctionclear gen [--profile <uniform|tie-heavy|market-heavy|extreme|mixed>] [--orders <n>] [--seed <n>]
  auctionclear venues";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, String> {
    match args.iter().position(|a| a == name) {
        None => Ok(None),
        Some(i) => args
            .get(i + 1)
            .map(|s| Some(s.as_str()))
            .ok_or_else(|| format!("{name} needs a value")),
    }
}

fn parse_num<T: std::str::FromStr>(value: &str, what: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid {what} {value:?}"))
}

fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("gen") => cmd_gen(&args[1..]),
        Some("venues") => Ok(cmd_venues()),
        Some("-h" | "--help" | "help") => Ok(format!("{USAGE}\n")),
        Some(other) => Err(format!("unknown command {other:?}")),
        None => Err("missing command".to_string()),
    }
}

fn cmd_venues() -> String {
    let mut out = String::new();
    for v in Venue::ALL {
        out.push_str(&format!(
            "{}\n  chain:          {}\n  rule text:      {}\n  interpretation: {}\n\n",
            v.name(),
            v.policy(),
            v.rule_text(),
            v.interpretation()
        ));
    }
    out
}

fn cmd_gen(args: &[String]) -> Result<String, String> {
    let profile = match flag_value(args, "--profile")? {
        None => Profile::TieHeavy,
        Some(p) => Profile::from_name(p).ok_or_else(|| format!("unknown profile {p:?}"))?,
    };
    let n: usize = flag_value(args, "--orders")?
        .map(|v| parse_num(v, "order count"))
        .transpose()?
        .unwrap_or(12);
    let seed: u64 = flag_value(args, "--seed")?
        .map(|v| parse_num(v, "seed"))
        .transpose()?
        .unwrap_or(1);
    let g = gen::generate(profile, n, &mut Rng::new(seed));
    Ok(csv::write(&g.orders, g.reference))
}

fn cmd_run(args: &[String]) -> Result<String, String> {
    let path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("run needs an orders file")?;
    let venue_name = flag_value(args, "--venue")?.ok_or("run needs --venue")?;
    let venue =
        Venue::from_name(venue_name).ok_or_else(|| format!("unknown venue {venue_name:?}"))?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let file = csv::parse(&text).map_err(|e| format!("{path}: {e}"))?;
    let collar = match flag_value(args, "--collar-bps")? {
        Some(v) => Some(Collar::new(parse_num::<u32>(
            v,
            "collar width in basis points",
        )?)),
        None => None,
    };
    let reference = match flag_value(args, "--reference")? {
        Some(v) => Some(parse_num::<i64>(v, "reference price")?),
        None => file.reference,
    };
    let book = Book::new(file.orders).map_err(|e| e.to_string())?;
    let policy = venue.policy();
    let outcome = book
        .uncross_with_collar(&policy, reference, collar)
        .map_err(|e| e.to_string())?;

    let mut out = String::new();
    out.push_str(&format!("venue      {} ({})\n", venue.name(), policy));
    out.push_str(&format!(
        "reference  {}\n",
        reference.map_or("none".to_string(), |r| r.to_string())
    ));

    if args.iter().any(|a| a == "--levels") {
        out.push_str("\ncandidates\n");
        out.push_str(&format!(
            "{:>12} {:>12} {:>12} {:>12} {:>12}  {}\n",
            "price", "demand", "supply", "volume", "imbalance", "eligible"
        ));
        for c in book.candidates(reference) {
            out.push_str(&format!(
                "{:>12} {:>12} {:>12} {:>12} {:>12}  {}\n",
                c.price,
                c.demand,
                c.supply,
                c.volume,
                c.imbalance,
                if c.eligible { "yes" } else { "no" }
            ));
        }
        out.push('\n');
    }

    match outcome {
        Outcome::NoTrade(reason) => {
            let why = match reason {
                NoTradeReason::NoCross => "no price has executable volume",
                NoTradeReason::NoReferencePrice => "only market orders and no reference price",
            };
            out.push_str(&format!("price      none ({why})\nvolume     0\n"));
        }
        Outcome::Extended {
            breach,
            volume,
            imbalance,
        } => {
            let pressure = match imbalance.signum() {
                1 => format!("{imbalance:+} (buy surplus)"),
                -1 => format!("{imbalance} (sell surplus)"),
                _ => "0".to_string(),
            };
            out.push_str("price      none (auction extended)\n");
            out.push_str(&format!("collar     {breach}\n"));
            out.push_str(&format!("indicative {}\n", breach.indicative));
            out.push_str(&format!("volume     {volume} (would have traded)\n"));
            out.push_str(&format!("imbalance  {pressure}\n"));
        }
        Outcome::Cleared(c) => {
            let imbalance = match c.imbalance.signum() {
                1 => format!("{:+} (buy surplus)", c.imbalance),
                -1 => format!("{} (sell surplus)", c.imbalance),
                _ => "0".to_string(),
            };
            out.push_str(&format!("price      {}\n", c.price));
            out.push_str(&format!("volume     {}\n", c.volume));
            out.push_str(&format!("imbalance  {imbalance}\n"));
            out.push_str(&format!(
                "\n{:>8} {:>5} {:>12} {:>10} {:>8} {:>10}\n",
                "id", "side", "limit", "qty", "time", "filled"
            ));
            for (o, filled) in book.orders().iter().zip(&c.fills) {
                let side = match o.side {
                    Side::Buy => "buy",
                    Side::Sell => "sell",
                };
                let limit = match o.limit {
                    Limit::Market => "market".to_string(),
                    Limit::Price(p) => p.to_string(),
                };
                out.push_str(&format!(
                    "{:>8} {:>5} {:>12} {:>10} {:>8} {:>10}\n",
                    o.id, side, limit, o.qty, o.time, filled
                ));
            }
        }
    }
    Ok(out)
}
