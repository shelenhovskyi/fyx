//! The bounded queue between the IO thread (parsed OSC events) and the main thread, and the rate
//! limits the IO thread applies before anything reaches it.

use std::collections::VecDeque;
use std::time::Instant;

use crate::osc::OscEvent;

/// Most events queued for one terminal before new ones are dropped.
pub const MAX_QUEUED_EVENTS: usize = 256;

/// Notifications (OSC 9, 99, 777): sustained rate and burst, per terminal.
pub const NOTIFY_RATE: Rate = Rate::new(4.0, 8.0);
/// Prompt marks and command lines (OSC 133, 633).
pub const SHELL_RATE: Rate = Rate::new(64.0, 128.0);
/// State events (cwd, progress, user vars, OSC 7501) coalesce in the queue; this only bounds parsing.
pub const STATE_RATE: Rate = Rate::new(64.0, 128.0);
/// Replies to the OSC 7501 support query; each one writes into the program's stdin.
pub const QUERY_REPLY_RATE: Rate = Rate::new(2.0, 4.0);

#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub per_second: f64,
    pub burst: f64,
}

impl Rate {
    pub const fn new(per_second: f64, burst: f64) -> Self {
        Self { per_second, burst }
    }
}

/// A token bucket, owned by the IO thread.
#[derive(Debug)]
pub struct RateLimiter {
    rate: Rate,
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            tokens: rate.burst,
            last: Instant::now(),
        }
    }

    pub fn allow(&mut self) -> bool {
        self.allow_at(Instant::now())
    }

    pub fn allow_at(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate.per_second).min(self.rate.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Inbox {
    events: VecDeque<OscEvent>,
    /// Events dropped by rate limits or a full queue since the last drain.
    dropped: u64,
}

impl Inbox {
    pub(crate) fn push(&mut self, event: OscEvent) {
        if let Some(key) = event.coalesce_key()
            && let Some(index) = self
                .events
                .iter()
                .position(|queued| queued.coalesce_key() == Some(key))
        {
            self.events.remove(index);
        } else if self.events.len() >= MAX_QUEUED_EVENTS {
            self.dropped += 1;
            return;
        }
        self.events.push_back(event);
    }

    pub(crate) fn note_dropped(&mut self) {
        self.dropped += 1;
    }

    pub(crate) fn drain(&mut self) -> (Vec<OscEvent>, u64) {
        (
            self.events.drain(..).collect(),
            std::mem::take(&mut self.dropped),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::osc::{Progress, Protocol};

    #[test]
    fn rate_limiter_allows_a_burst_then_the_rate() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(Rate::new(2.0, 3.0));
        limiter.last = start;
        assert_eq!(
            (0..5).filter(|_| limiter.allow_at(start)).count(),
            3,
            "burst"
        );
        assert!(!limiter.allow_at(start + Duration::from_millis(100)));
        assert!(limiter.allow_at(start + Duration::from_millis(600)));
        assert!(!limiter.allow_at(start + Duration::from_millis(650)));
    }

    #[test]
    fn state_events_coalesce_and_the_queue_is_bounded() {
        let mut inbox = Inbox::default();
        for value in 0..10 {
            inbox.push(OscEvent::Progress(Progress::Normal(value)));
        }
        inbox.push(OscEvent::Cwd {
            source: Protocol::Osc7,
            path: "/a".into(),
        });
        inbox.push(OscEvent::Progress(Progress::Remove));
        let (events, dropped) = inbox.drain();
        assert_eq!(
            events,
            [
                OscEvent::Cwd {
                    source: Protocol::Osc7,
                    path: "/a".into()
                },
                OscEvent::Progress(Progress::Remove)
            ]
        );
        assert_eq!(dropped, 0);

        for _ in 0..MAX_QUEUED_EVENTS + 5 {
            inbox.push(OscEvent::Notify {
                source: Protocol::Osc9,
                title: None,
                body: "x".into(),
            });
        }
        let (events, dropped) = inbox.drain();
        assert_eq!((events.len(), dropped), (MAX_QUEUED_EVENTS, 5));
    }
}
