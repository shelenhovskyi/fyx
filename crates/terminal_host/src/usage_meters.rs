//! Plan 7.8 (W3's part): cost and context meters. Each agent session reports its context use
//! (tokens used of the window) and its running cost; the meters keep the latest per session and
//! sum cost increases per local day, so "today" survives a session that ends. A soft daily cap
//! says when agents should be paused.
//!
//! The model is plain data: the ACP glue feeds it, the Agent Sessions panel draws it, and the
//! day totals are saved as JSON (`DayTotals`) by the caller.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// A session's key: unique among live sessions (an ACP session id, a terminal id).
pub type SessionKey = String;

/// What a session reported last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionUsage {
    pub label: String,
    /// Tokens in the context window now, and its size; `size` 0 when unknown.
    pub used_tokens: u64,
    pub window_tokens: u64,
    /// The session's running cost, as its agent reports it.
    pub cost: Option<Cost>,
}

impl SessionUsage {
    /// The share of the context window in use, 0 to 1; `None` without a window size.
    pub fn context_ratio(&self) -> Option<f32> {
        (self.window_tokens > 0)
            .then(|| (self.used_tokens as f64 / self.window_tokens as f64).min(1.0) as f32)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

impl Cost {
    /// "$1.24" for USD, "1.24 EUR" otherwise.
    pub fn text(&self) -> String {
        if self.currency.eq_ignore_ascii_case("USD") {
            format!("${:.2}", self.amount)
        } else {
            format!("{:.2} {}", self.amount, self.currency)
        }
    }
}

/// Cost per local day (`YYYY-MM-DD`) and currency: what is saved across restarts. Only the
/// last [`KEPT_DAYS`] days are kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DayTotals {
    pub days: BTreeMap<String, BTreeMap<String, f64>>,
}

pub const KEPT_DAYS: usize = 31;

/// Whether agents should pause: the day's cost in the cap's currency reached the cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CapState {
    NoCap,
    Under { spent: f64, cap: f64 },
    Reached { spent: f64, cap: f64 },
}

#[derive(Default)]
pub struct UsageMeters {
    sessions: HashMap<SessionKey, SessionUsage>,
    totals: DayTotals,
    /// The day the user chose to go on past the cap; no pause until the next day.
    cap_overridden_on: Option<String>,
}

impl UsageMeters {
    pub fn with_totals(totals: DayTotals) -> Self {
        Self {
            totals,
            ..Self::default()
        }
    }

    pub fn totals(&self) -> &DayTotals {
        &self.totals
    }

    pub fn session(&self, key: &str) -> Option<&SessionUsage> {
        self.sessions.get(key)
    }

    /// Records a session's report on `day`. A cost that grew adds the difference to the day; one
    /// that went down (a new conversation in the same session) counts again from zero. Returns
    /// whether the day's totals changed (the caller saves them).
    pub fn report(&mut self, key: &str, usage: SessionUsage, day: &str) -> bool {
        let previous = self
            .sessions
            .get(key)
            .and_then(|previous| previous.cost.clone());
        let added = match (&previous, &usage.cost) {
            (_, None) => None,
            (Some(previous), Some(cost)) if previous.currency == cost.currency => {
                if cost.amount >= previous.amount {
                    Some(cost.amount - previous.amount)
                } else {
                    Some(cost.amount)
                }
            }
            (_, Some(cost)) => Some(cost.amount),
        };
        let cost = usage.cost.clone();
        self.sessions.insert(key.to_string(), usage);
        match (added, cost) {
            (Some(added), Some(cost)) if added > 0.0 => {
                *self
                    .totals
                    .days
                    .entry(day.to_string())
                    .or_default()
                    .entry(cost.currency)
                    .or_default() += added;
                while self.totals.days.len() > KEPT_DAYS {
                    self.totals.days.pop_first();
                }
                true
            }
            _ => false,
        }
    }

    /// A session ended: its meter goes, its cost stays in the day.
    pub fn remove(&mut self, key: &str) {
        self.sessions.remove(key);
    }

    /// The day's cost per currency, sorted by currency.
    pub fn day(&self, day: &str) -> Vec<Cost> {
        self.totals
            .days
            .get(day)
            .map(|currencies| {
                currencies
                    .iter()
                    .map(|(currency, amount)| Cost {
                        amount: *amount,
                        currency: currency.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `cap` is in USD; other currencies do not count towards it.
    pub fn cap_state(&self, cap: Option<f64>, day: &str) -> CapState {
        let Some(cap) = cap.filter(|cap| *cap > 0.0) else {
            return CapState::NoCap;
        };
        let spent = self
            .day(day)
            .iter()
            .filter(|cost| cost.currency.eq_ignore_ascii_case("USD"))
            .map(|cost| cost.amount)
            .sum();
        if spent >= cap && self.cap_overridden_on.as_deref() != Some(day) {
            CapState::Reached { spent, cap }
        } else {
            CapState::Under { spent, cap }
        }
    }

    /// "Continue today": no pause for the rest of `day`.
    pub fn override_cap(&mut self, day: &str) {
        self.cap_overridden_on = Some(day.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(amount: Option<f64>) -> SessionUsage {
        SessionUsage {
            label: "Claude".into(),
            used_tokens: 50_000,
            window_tokens: 200_000,
            cost: amount.map(|amount| Cost {
                amount,
                currency: "USD".into(),
            }),
        }
    }

    #[test]
    fn test_cost_increases_sum_per_day() {
        let mut meters = UsageMeters::default();
        assert!(meters.report("a", usage(Some(0.5)), "2026-11-20"));
        assert!(meters.report("a", usage(Some(1.25)), "2026-11-20"));
        assert!(
            !meters.report("a", usage(Some(1.25)), "2026-11-20"),
            "unchanged"
        );
        assert!(meters.report("b", usage(Some(2.0)), "2026-11-20"));
        // A new conversation in session a starts from zero again.
        assert!(meters.report("a", usage(Some(0.25)), "2026-11-21"));
        meters.remove("b");
        assert_eq!(meters.day("2026-11-20")[0].text(), "$3.25");
        assert_eq!(meters.day("2026-11-21")[0].text(), "$0.25");
        assert_eq!(meters.session("a").unwrap().context_ratio(), Some(0.25));
        assert!(meters.session("b").is_none());
        let saved = serde_json::to_string(meters.totals()).unwrap();
        let restored = UsageMeters::with_totals(serde_json::from_str(&saved).unwrap());
        assert_eq!(restored.day("2026-11-20"), meters.day("2026-11-20"));
    }

    #[test]
    fn test_cap() {
        let mut meters = UsageMeters::default();
        let day = "2026-11-20";
        assert_eq!(meters.cap_state(None, day), CapState::NoCap);
        meters.report("a", usage(Some(4.0)), day);
        assert_eq!(
            meters.cap_state(Some(5.0), day),
            CapState::Under {
                spent: 4.0,
                cap: 5.0
            }
        );
        meters.report("a", usage(Some(5.5)), day);
        assert!(matches!(
            meters.cap_state(Some(5.0), day),
            CapState::Reached { .. }
        ));
        meters.override_cap(day);
        assert!(matches!(
            meters.cap_state(Some(5.0), day),
            CapState::Under { .. }
        ));
        meters.report("b", usage(Some(6.0)), "2026-11-21");
        assert!(matches!(
            meters.cap_state(Some(5.0), "2026-11-21"),
            CapState::Reached { .. }
        ));
    }
}
