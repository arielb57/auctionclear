use std::fmt;

/// Executable volume and imbalance at one candidate price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub price: i64,
    /// Buy quantity executable at this price (market bids plus bids at or above it).
    pub demand: u128,
    /// Sell quantity executable at this price (market asks plus asks at or below it).
    pub supply: u128,
    /// `min(demand, supply)`.
    pub volume: u128,
    /// `demand - supply`. Positive means buy pressure.
    pub imbalance: i128,
    /// True if, at this price, every limit order priced strictly through it
    /// can fill completely under price-time priority. See [`Step::Eligible`].
    pub eligible: bool,
}

/// One step of a tie-break chain.
///
/// Filter steps narrow the candidate set; terminal steps turn the remaining
/// set into a single price. A chain always starts with [`Step::MaxVolume`] and
/// ends with exactly one terminal step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// Keep the prices with the largest executable volume.
    MaxVolume,
    /// Keep the prices at which every bid above and every ask below the price
    /// fills completely. Market orders are exempt only when their side has no
    /// limit order priced through the candidate.
    Eligible,
    /// Keep the prices with the smallest `|demand - supply|`.
    MinAbsImbalance,
    /// If every remaining price has buy pressure keep the highest; if every one
    /// has sell pressure keep the lowest; otherwise keep them all.
    ImbalanceSide,
    /// Keep the prices closest to the reference price. No-op without one.
    NearestReference,
    /// Terminal: `floor((lowest + highest) / 2)` of the remaining prices.
    Midpoint,
    /// Terminal: the reference price clamped into `[lowest, highest]`, or the
    /// midpoint when no reference price is given.
    ClampReference,
    /// Terminal: the lowest remaining price.
    Lowest,
    /// Terminal: the highest remaining price.
    Highest,
}

impl Step {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Step::Midpoint | Step::ClampReference | Step::Lowest | Step::Highest
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Step::MaxVolume => "MaxVolume",
            Step::Eligible => "Eligible",
            Step::MinAbsImbalance => "MinAbsImbalance",
            Step::ImbalanceSide => "ImbalanceSide",
            Step::NearestReference => "NearestReference",
            Step::Midpoint => "Midpoint",
            Step::ClampReference => "ClampReference",
            Step::Lowest => "Lowest",
            Step::Highest => "Highest",
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    MustStartWithMaxVolume,
    MustEndWithTerminal,
    TerminalBeforeEnd(Step),
    RepeatedMaxVolume,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyError::MustStartWithMaxVolume => {
                write!(f, "a tie-break chain must start with MaxVolume")
            }
            PolicyError::MustEndWithTerminal => {
                write!(
                    f,
                    "a tie-break chain must end with Midpoint, ClampReference, Lowest or Highest"
                )
            }
            PolicyError::TerminalBeforeEnd(s) => {
                write!(f, "terminal step {s} must be the last step")
            }
            PolicyError::RepeatedMaxVolume => write!(f, "MaxVolume may only appear once, first"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// A validated tie-break chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    steps: Vec<Step>,
}

impl Policy {
    pub fn new(steps: Vec<Step>) -> Result<Self, PolicyError> {
        if steps.first() != Some(&Step::MaxVolume) {
            return Err(PolicyError::MustStartWithMaxVolume);
        }
        if !steps.last().is_some_and(|s| s.is_terminal()) {
            return Err(PolicyError::MustEndWithTerminal);
        }
        if steps[1..].contains(&Step::MaxVolume) {
            return Err(PolicyError::RepeatedMaxVolume);
        }
        if let Some(s) = steps[..steps.len() - 1].iter().find(|s| s.is_terminal()) {
            return Err(PolicyError::TerminalBeforeEnd(*s));
        }
        Ok(Policy { steps })
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Runs the chain over `candidates` (sorted by ascending price, non-empty).
    ///
    /// Returns the chosen price, or the filter step that emptied the set.
    pub(crate) fn choose(
        &self,
        candidates: &[Candidate],
        reference: Option<i64>,
    ) -> Result<i64, Step> {
        let mut set: Vec<Candidate> = candidates.to_vec();
        for &step in &self.steps {
            match step {
                Step::MaxVolume => {
                    let best = set.iter().map(|c| c.volume).max().unwrap_or(0);
                    set.retain(|c| c.volume == best);
                }
                Step::Eligible => set.retain(|c| c.eligible),
                Step::MinAbsImbalance => {
                    let best = set
                        .iter()
                        .map(|c| c.imbalance.unsigned_abs())
                        .min()
                        .unwrap_or(0);
                    set.retain(|c| c.imbalance.unsigned_abs() == best);
                }
                Step::ImbalanceSide => {
                    if !set.is_empty() && set.iter().all(|c| c.imbalance > 0) {
                        set = vec![set[set.len() - 1]];
                    } else if !set.is_empty() && set.iter().all(|c| c.imbalance < 0) {
                        set = vec![set[0]];
                    }
                }
                Step::NearestReference => {
                    if let Some(r) = reference {
                        let dist = |c: &Candidate| (c.price as i128 - r as i128).unsigned_abs();
                        let best = set.iter().map(dist).min().unwrap_or(0);
                        set.retain(|c| dist(c) == best);
                    }
                }
                Step::Midpoint | Step::ClampReference | Step::Lowest | Step::Highest => {
                    let (Some(lo), Some(hi)) = (set.first(), set.last()) else {
                        return Err(step);
                    };
                    let (lo, hi) = (lo.price, hi.price);
                    return Ok(match (step, reference) {
                        (Step::Lowest, _) => lo,
                        (Step::Highest, _) => hi,
                        (Step::ClampReference, Some(r)) => r.clamp(lo, hi),
                        _ => midpoint(lo, hi),
                    });
                }
            }
            if set.is_empty() {
                return Err(step);
            }
        }
        unreachable!("Policy::new guarantees a terminal step")
    }
}

impl fmt::Display for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.steps.iter().map(|s| s.name()).collect();
        f.write_str(&names.join(" -> "))
    }
}

/// `floor((lo + hi) / 2)` without overflow.
pub fn midpoint(lo: i64, hi: i64) -> i64 {
    (lo as i128 + hi as i128).div_euclid(2) as i64
}

/// Venue-style presets. Each one paraphrases a published rule and lists which
/// of its steps are this crate's interpretation rather than rule text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Venue {
    Sse,
    Szse,
    Nasdaq,
    Xetra,
}

impl Venue {
    pub const ALL: [Venue; 4] = [Venue::Sse, Venue::Szse, Venue::Nasdaq, Venue::Xetra];

    pub fn name(self) -> &'static str {
        match self {
            Venue::Sse => "sse",
            Venue::Szse => "szse",
            Venue::Nasdaq => "nasdaq",
            Venue::Xetra => "xetra",
        }
    }

    pub fn from_name(name: &str) -> Option<Venue> {
        Venue::ALL
            .into_iter()
            .find(|v| v.name().eq_ignore_ascii_case(name))
    }

    pub fn steps(self) -> Vec<Step> {
        use Step::*;
        match self {
            Venue::Sse => vec![MaxVolume, Eligible, MinAbsImbalance, Midpoint],
            Venue::Szse => vec![
                MaxVolume,
                Eligible,
                MinAbsImbalance,
                NearestReference,
                Midpoint,
            ],
            Venue::Nasdaq => vec![
                MaxVolume,
                Eligible,
                MinAbsImbalance,
                NearestReference,
                Lowest,
            ],
            Venue::Xetra => vec![
                MaxVolume,
                MinAbsImbalance,
                ImbalanceSide,
                Eligible,
                ClampReference,
            ],
        }
    }

    pub fn policy(self) -> Policy {
        Policy::new(self.steps()).expect("presets are valid chains")
    }

    /// Paraphrase of the published rule the preset follows.
    pub fn rule_text(self) -> &'static str {
        match self {
            Venue::Sse => {
                "Shanghai Stock Exchange Trading Rules, call auction price determination: the price \
                 that (1) achieves the maximum executed volume, (2) fully executes all bids above and \
                 all asks below it, and (3) fully executes at least one side of the orders at it. If \
                 several prices qualify, the one leaving the least unexecuted volume; if several \
                 still qualify, their midpoint."
            }
            Venue::Szse => {
                "Shenzhen Stock Exchange Trading Rules, call auction price determination: the same \
                 three conditions as Shanghai. If several prices qualify, the one with the smallest \
                 difference between cumulative buy and sell quantity; if several still qualify, the \
                 one closest to the reference price (previous close for the opening auction)."
            }
            Venue::Nasdaq => {
                "Nasdaq Rule 4752 (Opening Cross) and 4754 (Closing Cross), price determination: the \
                 price that maximises executed shares; if several, the one that minimises the \
                 imbalance; if several, the one closest to the cross reference price."
            }
            Venue::Xetra => {
                "Deutsche Boerse Xetra market model, auction price determination: the price with the \
                 highest executable volume and the lowest surplus; if several, the highest limit when \
                 every candidate has a bid surplus and the lowest limit when every candidate has an \
                 ask surplus; otherwise the reference price, or the candidate limit nearest to it."
            }
        }
    }

    /// Steps of the preset that are not stated in the rule text.
    pub fn interpretation(self) -> &'static str {
        match self {
            Venue::Sse => {
                "MinAbsImbalance reads 'least unexecuted volume' as the smallest |demand - supply| at \
                 the price. Midpoint rounds down to a whole tick."
            }
            Venue::Szse => {
                "Midpoint is not in the rule text: it resolves two candidates equidistant from the \
                 reference, and then lands on the reference price itself."
            }
            Venue::Nasdaq => {
                "Eligible is not in the rule text; without it a cross can leave an order priced \
                 through the cross price partially filled. Lowest resolves two candidates equidistant \
                 from the reference."
            }
            Venue::Xetra => {
                "Eligible is not in the rule text; without it a surplus of market orders can crowd out \
                 a limit order priced through the reference price. Candidates are the limit prices in \
                 the book, and the reference price becomes the auction price only through \
                 ClampReference. With no reference price the midpoint is used."
            }
        }
    }
}
