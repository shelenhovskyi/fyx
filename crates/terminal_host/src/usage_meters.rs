//! Plan 7.8 (W3's part): cost and context meters. Each agent session reports its context use
//! (tokens used of the window) and its running cost; the meters keep the latest per session and
//! sum cost increases per local day, so "today" survives a session that ends. A soft daily cap
//! says when agents should be paused.
//!
//! The model is plain data: the ACP glue feeds it, the Agent Sessions panel draws it, and the
//! day totals are saved as JSON (`DayTotals`) by the caller.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use acp_thread::{AcpThread, AcpThreadEvent, ThreadStatus};
use gpui::{App, AppContext as _, Context, DismissEvent, Entity, EntityId, Global, Subscription};
use serde::{Deserialize, Serialize};
use settings::Settings as _;
use workspace::notifications::{
    NotificationId, show_app_notification, simple_message_notification::MessageNotification,
};

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

/// The UTC day of `time` as `YYYY-MM-DD` (days are UTC: there is no time-zone data here).
pub fn day_of(time: std::time::SystemTime) -> String {
    let days = time
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 86_400) as i64;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn today() -> String {
    day_of(std::time::SystemTime::now())
}

// The live meters: every ACP thread's usage and cost, and the day totals saved in a file.

struct Watched {
    key: SessionKey,
    _subscription: Subscription,
}

/// The app's meters, fed by every [`AcpThread`]; the Agent Sessions panel observes it.
pub struct LiveMeters {
    meters: UsageMeters,
    file: Option<PathBuf>,
    threads: HashMap<EntityId, Watched>,
    /// The cap notice shows once per day.
    noticed_on: Option<String>,
}

struct GlobalMeters(Entity<LiveMeters>);

impl Global for GlobalMeters {}

/// The meters, once [`init`] ran.
pub fn global(cx: &App) -> Option<Entity<LiveMeters>> {
    cx.try_global::<GlobalMeters>()
        .map(|global| global.0.clone())
}

/// Starts the meters; day totals live in `data_dir/agent_usage.json` (`None` keeps them in
/// memory, for tests).
pub fn init(data_dir: Option<PathBuf>, cx: &mut App) {
    let file = data_dir.map(|dir| dir.join("agent_usage.json"));
    let totals = file
        .as_ref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str::<DayTotals>(&text).ok())
        .unwrap_or_default();
    let meters = cx.new(|_| LiveMeters {
        meters: UsageMeters::with_totals(totals),
        file,
        threads: HashMap::default(),
        noticed_on: None,
    });
    cx.set_global(GlobalMeters(meters.clone()));
    cx.observe_new(move |_: &mut AcpThread, _, cx: &mut Context<AcpThread>| {
        let thread = cx.entity();
        let meters = meters.clone();
        cx.defer(move |cx| meters.update(cx, |meters, cx| meters.watch(thread, cx)));
    })
    .detach();
}

impl LiveMeters {
    pub fn meters(&self) -> &UsageMeters {
        &self.meters
    }

    /// The meter of a live thread.
    pub fn thread_usage(&self, thread: EntityId) -> Option<&SessionUsage> {
        let watched = self.threads.get(&thread)?;
        self.meters.session(&watched.key)
    }

    pub fn today(&self) -> Vec<Cost> {
        self.meters.day(&today())
    }

    pub fn cap_state(&self, cx: &App) -> CapState {
        self.meters.cap_state(cap(cx), &today())
    }

    /// "Continue Today" on the cap notice.
    pub fn continue_today(&mut self, cx: &mut Context<Self>) {
        self.meters.override_cap(&today());
        cx.notify();
    }

    fn watch(&mut self, thread: Entity<AcpThread>, cx: &mut Context<Self>) {
        let id = thread.entity_id();
        let subscription = cx.subscribe(&thread, |meters, thread, event, cx| match event {
            AcpThreadEvent::TokenUsageUpdated(_) => meters.report(&thread, cx),
            _ => meters.enforce_cap(&thread, cx),
        });
        cx.observe_release(&thread, move |meters, _, cx| {
            if let Some(watched) = meters.threads.remove(&id) {
                meters.meters.remove(&watched.key);
                cx.notify();
            }
        })
        .detach();
        let key = format!("acp:{}", thread.read(cx).session_id());
        self.threads.insert(
            id,
            Watched {
                key,
                _subscription: subscription,
            },
        );
        self.report(&thread, cx);
    }

    fn report(&mut self, thread: &Entity<AcpThread>, cx: &mut Context<Self>) {
        let Some(key) = self
            .threads
            .get(&thread.entity_id())
            .map(|watched| watched.key.clone())
        else {
            return;
        };
        let read = thread.read(cx);
        let usage = read.token_usage();
        let report = SessionUsage {
            label: read
                .title()
                .map_or_else(|| "Agent".to_string(), |title| title.to_string()),
            used_tokens: usage.map_or(0, |usage| usage.used_tokens),
            window_tokens: usage.map_or(0, |usage| usage.max_tokens),
            cost: read.cost().map(|cost| Cost {
                amount: cost.amount,
                currency: cost.currency.to_string(),
            }),
        };
        if self.meters.report(&key, report, &today()) {
            self.save(cx);
        }
        cx.notify();
        self.enforce_cap(thread, cx);
    }

    fn save(&self, cx: &mut Context<Self>) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let text = match serde_json::to_string(self.meters.totals()) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("fyx: agent usage totals: {error}");
                return;
            }
        };
        cx.background_spawn(async move {
            if let Err(error) = std::fs::write(&file, text) {
                log::warn!("fyx: saving {}: {error}", file.display());
            }
        })
        .detach();
    }

    /// Over the cap: a generating thread is stopped, and the notice shows once a day.
    fn enforce_cap(&mut self, thread: &Entity<AcpThread>, cx: &mut Context<Self>) {
        let CapState::Reached { spent, cap } = self.cap_state(cx) else {
            return;
        };
        if thread.read(cx).status() != ThreadStatus::Generating {
            return;
        }
        thread.update(cx, |thread, cx| thread.cancel(cx)).detach();
        let day = today();
        if self.noticed_on.as_deref() == Some(day.as_str()) {
            return;
        }
        self.noticed_on = Some(day);
        let meters = cx.entity().downgrade();
        let message = format!(
            "Agents paused: today's cost ${spent:.2} reached the daily cap of ${cap:.2} \
             (fyx.terminal.daily_cost_cap)."
        );
        show_app_notification(NotificationId::unique::<LiveMeters>(), cx, move |cx| {
            let meters = meters.clone();
            let message = message.clone();
            cx.new(|cx| {
                MessageNotification::new(message, cx)
                    .primary_message("Continue Today")
                    .primary_on_click(move |_, cx| {
                        meters
                            .update(cx, |meters, cx| meters.continue_today(cx))
                            .ok();
                        cx.emit(DismissEvent);
                    })
            })
        });
    }
}

fn cap(cx: &App) -> Option<f64> {
    if !cx.has_global::<settings::SettingsStore>() {
        return None;
    }
    crate::settings::FyxTerminalSettings::get_global(cx).daily_cost_cap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_day_of() {
        let at = |seconds| std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        assert_eq!(day_of(at(0)), "1970-01-01");
        assert_eq!(day_of(at(951_782_400)), "2000-02-29");
        assert_eq!(day_of(at(1_795_132_800)), "2026-11-20");
    }

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
