//! The tick rules of design.md, B, as plain state machines with no timers or GPUI types, for the
//! renderer to drive: it knows when a panel is visible and whether a Fyx window is active, and it
//! owns the executor timers. `request-tick` itself (the 100 ms floor, the 24 h cap, the surface
//! checks) is checked by the host import ([`crate::HostEffect::RequestTick`]).
//!
//! | Rule | Type |
//! |---|---|
//! | 100 ms floor; 1000 ms while no Fyx window is active; deferred while the panel is hidden, then delivered once | [`PendingTick`] |
//! | 20 ticks per second per extension, excess delayed | [`TickBudget::reserve`] |
//! | more than 10 % of wall time over 10 s clamps every tick of the extension to 1000 ms for 10 s | [`TickBudget::record`] |
//! | after 3 consecutive `tick` errors on a surface, its ticks stop until the next user event | [`TickErrors`] |

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub use crate::effects::{MAX_TICK_DELAY, MIN_TICK_DELAY};

/// Minimum tick delay while no Fyx window is active.
pub const BACKGROUND_TICK_DELAY: Duration = Duration::from_secs(1);
/// Ticks per second one extension may run across all its surfaces; a burst of this many is
/// allowed.
pub const TICKS_PER_SECOND: u32 = 20;
/// The window over which an extension's tick CPU time is measured.
pub const TICK_CPU_WINDOW: Duration = Duration::from_secs(10);
/// Tick time above this share of [`TICK_CPU_WINDOW`] throttles the extension.
pub const TICK_CPU_SHARE_PERCENT: u32 = 10;
/// Minimum tick delay of a throttled extension, for [`TICK_CPU_WINDOW`].
pub const THROTTLED_TICK_DELAY: Duration = Duration::from_secs(1);
/// Consecutive `tick` errors after which a surface's ticks stop until a user event on it.
pub const MAX_CONSECUTIVE_TICK_ERRORS: u32 = 3;

/// What the renderer knows about a surface when it schedules its tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickConditions {
    /// A panel shown in some window (dock open and the panel active), a status item, a shown
    /// modal, a decoration in an open editor.
    pub visible: bool,
    /// Some Fyx window is active.
    pub app_active: bool,
    /// The extension's minimum delay from [`TickBudget::min_delay`].
    pub extension_min_delay: Duration,
}

/// One surface's pending tick: one at most, a new request replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingTick {
    requested_at: Instant,
    delay: Duration,
}

impl PendingTick {
    /// A tick `delay` after `applied_at`, the moment the requesting call's effects were applied.
    pub fn new(delay: Duration, applied_at: Instant) -> Self {
        Self {
            requested_at: applied_at,
            delay: delay.clamp(MIN_TICK_DELAY, MAX_TICK_DELAY),
        }
    }

    /// When to deliver the tick, or `None` while the surface is hidden. A tick that fell due while
    /// hidden is due at once when the surface shows again: one tick, never a backlog, because
    /// there is only one pending tick per surface.
    pub fn due(&self, conditions: TickConditions) -> Option<Instant> {
        if !conditions.visible {
            return None;
        }
        let mut delay = self.delay.max(conditions.extension_min_delay);
        if !conditions.app_active {
            delay = delay.max(BACKGROUND_TICK_DELAY);
        }
        Some(self.requested_at + delay)
    }
}

/// The per-extension rate limits: a token bucket over every surface's ticks and the CPU-share
/// throttle. Keep one per extension id.
#[derive(Clone, Debug)]
pub struct TickBudget {
    /// The theoretical arrival time of the generic cell rate algorithm: an equivalent of a token
    /// bucket with [`TICKS_PER_SECOND`] tokens refilled at that rate.
    next_arrival: Option<Instant>,
    /// Finish time and duration of each tick call in the last [`TICK_CPU_WINDOW`].
    calls: VecDeque<(Instant, Duration)>,
    throttled_until: Option<Instant>,
}

impl Default for TickBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl TickBudget {
    pub fn new() -> Self {
        Self {
            next_arrival: None,
            calls: VecDeque::new(),
            throttled_until: None,
        }
    }

    fn interval() -> Duration {
        Duration::from_secs(1) / TICKS_PER_SECOND
    }

    /// Takes a token for a tick that is due at `due`, and returns when it may actually run: `due`,
    /// or later when the extension already ran [`TICKS_PER_SECOND`] ticks in the last second.
    /// Excess ticks are delayed, never dropped.
    pub fn reserve(&mut self, due: Instant) -> Instant {
        let interval = Self::interval();
        let tolerance = interval * (TICKS_PER_SECOND - 1);
        let arrival = self.next_arrival.map_or(due, |next| next.max(due));
        let run_at = arrival
            .checked_sub(tolerance)
            .map_or(due, |earliest| earliest.max(due));
        self.next_arrival = Some(arrival.max(run_at) + interval);
        run_at
    }

    /// Records a finished tick call. Returns `true` when this call throttles the extension, which
    /// the renderer reports with one `warn` log.
    pub fn record(&mut self, finished_at: Instant, busy: Duration) -> bool {
        self.calls.push_back((finished_at, busy));
        while let Some(&(at, _)) = self.calls.front() {
            if finished_at.saturating_duration_since(at) > TICK_CPU_WINDOW {
                self.calls.pop_front();
            } else {
                break;
            }
        }
        if self.is_throttled(finished_at) {
            return false;
        }
        let busy: Duration = self.calls.iter().map(|&(_, busy)| busy).sum();
        if busy * 100 > TICK_CPU_WINDOW * TICK_CPU_SHARE_PERCENT {
            self.throttled_until = Some(finished_at + TICK_CPU_WINDOW);
            self.calls.clear();
            true
        } else {
            false
        }
    }

    pub fn is_throttled(&self, now: Instant) -> bool {
        self.throttled_until.is_some_and(|until| now < until)
    }

    /// The least delay of every tick of the extension at `now`.
    pub fn min_delay(&self, now: Instant) -> Duration {
        if self.is_throttled(now) {
            THROTTLED_TICK_DELAY
        } else {
            MIN_TICK_DELAY
        }
    }
}

/// Consecutive `tick` errors of one surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickErrors {
    consecutive: u32,
}

impl TickErrors {
    /// A `tick` call failed. Returns `true` when the surface's ticks stop now.
    pub fn on_error(&mut self) -> bool {
        self.consecutive += 1;
        self.consecutive == MAX_CONSECUTIVE_TICK_ERRORS
    }

    pub fn on_success(&mut self) {
        self.consecutive = 0;
    }

    /// A user event on the surface lets its ticks resume.
    pub fn on_user_event(&mut self) {
        self.consecutive = 0;
    }

    /// Ticks for the surface are not delivered (nor requested ticks scheduled) while stopped.
    pub fn stopped(&self) -> bool {
        self.consecutive >= MAX_CONSECUTIVE_TICK_ERRORS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: TickConditions = TickConditions {
        visible: true,
        app_active: true,
        extension_min_delay: MIN_TICK_DELAY,
    };

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn test_pending_tick_floor_background_and_hidden() {
        let now = Instant::now();
        assert_eq!(
            PendingTick::new(ms(10), now).due(ACTIVE),
            Some(now + ms(100))
        );
        assert_eq!(
            PendingTick::new(ms(250), now).due(ACTIVE),
            Some(now + ms(250))
        );
        let background = TickConditions {
            app_active: false,
            ..ACTIVE
        };
        assert_eq!(
            PendingTick::new(ms(250), now).due(background),
            Some(now + ms(1000))
        );
        assert_eq!(
            PendingTick::new(ms(5000), now).due(background),
            Some(now + ms(5000))
        );
        let throttled = TickConditions {
            extension_min_delay: THROTTLED_TICK_DELAY,
            ..ACTIVE
        };
        assert_eq!(
            PendingTick::new(ms(100), now).due(throttled),
            Some(now + ms(1000))
        );

        // Hidden: not due. Shown again hours later: due in the past, so delivered at once, once.
        let tick = PendingTick::new(ms(100), now);
        let hidden = TickConditions {
            visible: false,
            ..ACTIVE
        };
        assert_eq!(tick.due(hidden), None);
        assert!(tick.due(ACTIVE).expect("visible") < now + Duration::from_secs(3600));
    }

    #[test]
    fn test_budget_delays_the_21st_tick_in_a_second() {
        let now = Instant::now();
        let mut budget = TickBudget::new();
        for _ in 0..TICKS_PER_SECOND {
            assert_eq!(budget.reserve(now), now);
        }
        let delayed = budget.reserve(now);
        assert!(
            delayed > now && delayed <= now + ms(50),
            "{:?}",
            delayed - now
        );
        // A second later the bucket is full again.
        let later = now + Duration::from_secs(2);
        assert_eq!(budget.reserve(later), later);
    }

    #[test]
    fn test_cpu_share_throttles_for_ten_seconds() {
        let now = Instant::now();
        let mut budget = TickBudget::new();
        // 9 × 110 ms = 990 ms busy in 10 s: under 10 %.
        for index in 0..9 {
            assert!(!budget.record(now + ms(index * 100), ms(110)));
        }
        assert_eq!(budget.min_delay(now + ms(900)), MIN_TICK_DELAY);
        // The tenth pushes it past 1 s: throttled, reported once.
        assert!(budget.record(now + ms(1000), ms(110)));
        assert!(!budget.record(now + ms(1100), ms(110)));
        assert_eq!(budget.min_delay(now + ms(1100)), THROTTLED_TICK_DELAY);
        assert_eq!(budget.min_delay(now + ms(11_001)), MIN_TICK_DELAY);
    }

    #[test]
    fn test_ticks_stop_after_three_errors_until_a_user_event() {
        let mut errors = TickErrors::default();
        assert!(!errors.on_error());
        errors.on_success();
        assert!(!errors.on_error());
        assert!(!errors.on_error());
        assert!(!errors.stopped());
        assert!(errors.on_error());
        assert!(errors.stopped());
        assert!(!errors.on_error(), "reported once");
        errors.on_user_event();
        assert!(!errors.stopped());
    }
}
