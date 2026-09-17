//! Order file format: `id,side,price,qty,time`, one order per line.
//!
//! `side` is `buy`/`sell` (or `b`/`s`); `price` is an integer tick or `market`.
//! A header line starting with `id` is skipped, blank lines and `#` comments
//! are ignored, and a `# reference: <ticks>` comment sets the reference price.

use std::fmt;

use crate::order::{Limit, Order, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderFile {
    pub orders: Vec<Order>,
    pub reference: Option<i64>,
}

pub fn parse(text: &str) -> Result<OrderFile, ParseError> {
    let mut orders = Vec::new();
    let mut reference = None;
    for (n, raw) in text.lines().enumerate() {
        let line_no = n + 1;
        let line = raw.trim();
        let err = |message: String| ParseError {
            line: line_no,
            message,
        };
        if let Some(comment) = line.strip_prefix('#') {
            if let Some(value) = comment.trim().strip_prefix("reference:") {
                let value = value.trim();
                reference = Some(
                    value
                        .parse::<i64>()
                        .map_err(|_| err(format!("invalid reference price {value:?}")))?,
                );
            }
            continue;
        }
        if line.is_empty() || (orders.is_empty() && line.to_ascii_lowercase().starts_with("id")) {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() != 5 {
            return Err(err(format!(
                "expected 5 fields (id,side,price,qty,time), found {}",
                fields.len()
            )));
        }
        let id = fields[0]
            .parse::<u64>()
            .map_err(|_| err(format!("invalid id {:?}", fields[0])))?;
        let side = match fields[1].to_ascii_lowercase().as_str() {
            "buy" | "b" => Side::Buy,
            "sell" | "s" => Side::Sell,
            other => return Err(err(format!("invalid side {other:?}, expected buy or sell"))),
        };
        let limit = match fields[2].to_ascii_lowercase().as_str() {
            "market" | "mkt" => Limit::Market,
            p => Limit::Price(
                p.parse::<i64>()
                    .map_err(|_| err(format!("invalid price {p:?}")))?,
            ),
        };
        let qty = fields[3]
            .parse::<u64>()
            .map_err(|_| err(format!("invalid quantity {:?}", fields[3])))?;
        if qty == 0 {
            return Err(err("quantity must be positive".to_string()));
        }
        let time = fields[4]
            .parse::<u64>()
            .map_err(|_| err(format!("invalid time {:?}", fields[4])))?;
        orders.push(Order {
            id,
            side,
            limit,
            qty,
            time,
        });
    }
    Ok(OrderFile { orders, reference })
}

pub fn write(orders: &[Order], reference: Option<i64>) -> String {
    let mut out = String::new();
    if let Some(r) = reference {
        out.push_str(&format!("# reference: {r}\n"));
    }
    out.push_str("id,side,price,qty,time\n");
    for o in orders {
        let side = match o.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let price = match o.limit {
            Limit::Market => "market".to_string(),
            Limit::Price(p) => p.to_string(),
        };
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            o.id, side, price, o.qty, o.time
        ));
    }
    out
}
