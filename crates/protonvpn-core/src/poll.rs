//! Polling policy — `docs/architecture.md` §7.
//!
//! Three ways a status read happens, and no others:
//!
//! * **idle cadence**, never more often than once per 5 minutes,
//! * **immediately after an invocation that could have changed the connection**,
//! * **attention-driven**: the user opened the window or clicked the tray.
//!
//! Deleting the third one is how you get a UI that lies for five minutes after the user ran
//! `protonvpn connect` in their own terminal; deleting the second makes our own actions feel
//! broken. The first is what keeps a ~1 s Python spawn from becoming a battery complaint.
//!
//! The clock is injected, so the whole policy is tested without sleeping.

use std::time::{Duration, Instant};

/// Idle cadence. At ~1 s per `protonvpn status`, this is a ~0.3 % duty cycle.
pub const IDLE_INTERVAL: Duration = Duration::from_secs(300);

/// Attention polls are user-initiated and can be repeated. Refusing to re-poll within this window
/// keeps a fidgeting user from spawning a status process per click, at the cost of nothing: the
/// displayed age is then at most this large, and the age is shown.
pub const ATTENTION_FLOOR: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct PollSchedule {
    idle: Duration,
    attention_floor: Duration,
    last_poll: Option<Instant>,
}

impl Default for PollSchedule {
    fn default() -> Self {
        Self::new(IDLE_INTERVAL, ATTENTION_FLOOR)
    }
}

impl PollSchedule {
    pub fn new(idle: Duration, attention_floor: Duration) -> Self {
        Self {
            idle,
            attention_floor,
            last_poll: None,
        }
    }

    pub fn last_poll(&self) -> Option<Instant> {
        self.last_poll
    }

    pub fn note_poll(&mut self, now: Instant) {
        self.last_poll = Some(now);
    }

    /// Has the idle timer expired? True before any poll has happened.
    pub fn idle_due(&self, now: Instant) -> bool {
        match self.last_poll {
            None => true,
            Some(last) => now.duration_since(last) >= self.idle,
        }
    }

    /// Should an attention event (window opened, tray clicked) trigger a read?
    pub fn attention_due(&self, now: Instant) -> bool {
        match self.last_poll {
            None => true,
            Some(last) => now.duration_since(last) >= self.attention_floor,
        }
    }

    /// An attention event happened. Returns whether it should cause a poll, and records it.
    pub fn on_attention(&mut self, now: Instant) -> bool {
        if self.attention_due(now) {
            self.note_poll(now);
            true
        } else {
            false
        }
    }

    /// An invocation finished. `changed_state` means it could have moved the tunnel, in which case
    /// the read is immediate — the user is owed the result of the thing they just asked for.
    pub fn on_invocation_finished(&mut self, now: Instant, changed_state: bool) -> bool {
        if changed_state {
            self.note_poll(now);
            true
        } else {
            false
        }
    }

    /// When the idle timer next comes due, for the engine's sleep computation.
    pub fn next_idle_deadline(&self) -> Option<Instant> {
        self.last_poll.map(|last| last + self.idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule() -> (PollSchedule, Instant) {
        let start = Instant::now();
        (
            PollSchedule::new(Duration::from_secs(300), Duration::from_secs(20)),
            start,
        )
    }

    #[test]
    fn polls_immediately_before_anything_has_been_read() {
        let (schedule, now) = schedule();
        assert!(schedule.idle_due(now));
        assert!(schedule.attention_due(now));
    }

    #[test]
    fn the_idle_cadence_is_five_minutes_and_not_a_second_less() {
        let (mut schedule, now) = schedule();
        schedule.note_poll(now);
        assert!(!schedule.idle_due(now + Duration::from_secs(299)));
        assert!(schedule.idle_due(now + Duration::from_secs(300)));
        assert_eq!(
            schedule.next_idle_deadline(),
            Some(now + Duration::from_secs(300))
        );
    }

    #[test]
    fn an_attention_poll_is_not_a_timer() {
        let (mut schedule, now) = schedule();
        schedule.note_poll(now);
        // A click one second later does not spawn another Python process…
        assert!(!schedule.on_attention(now + Duration::from_secs(1)));
        // …but one after the floor does, without waiting for the idle cadence.
        assert!(!schedule.idle_due(now + Duration::from_secs(21)));
        assert!(schedule.on_attention(now + Duration::from_secs(21)));
        // And having polled, the window restarts.
        assert!(!schedule.on_attention(now + Duration::from_secs(22)));
    }

    #[test]
    fn a_state_changing_invocation_forces_an_immediate_read() {
        let (mut schedule, now) = schedule();
        schedule.note_poll(now);
        assert!(!schedule.idle_due(now + Duration::from_millis(500)));
        assert!(schedule.on_invocation_finished(now + Duration::from_millis(500), true));
    }

    #[test]
    fn a_harmless_invocation_does_not() {
        let (mut schedule, now) = schedule();
        schedule.note_poll(now);
        assert!(!schedule.on_invocation_finished(now + Duration::from_secs(1), false));
        // The idle timer is untouched by it.
        assert!(schedule.idle_due(now + Duration::from_secs(300)));
    }
}
