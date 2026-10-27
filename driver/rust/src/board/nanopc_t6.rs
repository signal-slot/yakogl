// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! When the NanoPC-T6 glue still feeds WDT0
//! (driver/zephyr/lib/board/nanopc_t6/nanopc_t6.c, its 1 s timer;
//! driver/docs/nanopc-t6-mali.md, "Z1"). The C side owns the timer and the
//! clock; this decides.
//!
//! The rule: feed only before the lifetime is over, only while the sample
//! has not ended the run (`released`), and, once the sample has said it is
//! alive (`t6_watchdog_feed()`), only while its last word is at most the
//! alive window old. When the feeding stops, WDT0 resets the SoC within one
//! period.

/// The feeder's verdict for one tick. The value crosses to C
/// (`YD_T6_FEED_*`), one number per cause, so the stop line names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Feed {
    /// Feed WDT0.
    Yes = 0,
    /// `t6_reset_to_linux()` ended the run.
    Released = 1,
    /// The uptime reached the lifetime (`deadline_ms`).
    LifetimeOver = 2,
    /// The sample's last `t6_watchdog_feed()` is older than the alive window.
    NoProgress = 3,
}

/// The verdict at uptime `now_ms`. `alive_at_ms` is the uptime of the
/// sample's last `t6_watchdog_feed()`, negative when it has not called it
/// yet. The deadline is exclusive (at `deadline_ms` the feeding stops), the
/// alive window inclusive (a call exactly `alive_ms` ago still feeds).
pub fn feed_verdict(now_ms: i64, alive_at_ms: i64, released: bool, deadline_ms: u32, alive_ms: u32) -> Feed {
    if released {
        Feed::Released
    } else if now_ms >= i64::from(deadline_ms) {
        Feed::LifetimeOver
    } else if alive_at_ms >= 0 && now_ms - alive_at_ms > i64::from(alive_ms) {
        Feed::NoProgress
    } else {
        Feed::Yes
    }
}

/// Whether the tick at `now_ms` feeds WDT0 ([`feed_verdict`] is [`Feed::Yes`]).
pub fn may_feed(now_ms: i64, alive_at_ms: i64, released: bool, deadline_ms: u32, alive_ms: u32) -> bool {
    feed_verdict(now_ms, alive_at_ms, released, deadline_ms, alive_ms) == Feed::Yes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The image's defaults: CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S 60,
    /// CONFIG_YAKOGL_NANOPC_T6_ALIVE_S 5.
    const DEADLINE: u32 = 60_000;
    const ALIVE: u32 = 5_000;

    #[test]
    fn before_the_sample_speaks_it_feeds_until_the_deadline_and_not_at_it() {
        assert!(may_feed(0, -1, false, DEADLINE, ALIVE));
        assert!(may_feed(59_999, -1, false, DEADLINE, ALIVE));
        assert_eq!(feed_verdict(60_000, -1, false, DEADLINE, ALIVE), Feed::LifetimeOver);
        assert_eq!(feed_verdict(60_001, -1, false, DEADLINE, ALIVE), Feed::LifetimeOver);
    }

    #[test]
    fn the_alive_window_is_inclusive() {
        assert!(may_feed(15_000, 10_000, false, DEADLINE, ALIVE));
        assert_eq!(feed_verdict(15_001, 10_000, false, DEADLINE, ALIVE), Feed::NoProgress);
        // a call at uptime 0 counts as a call
        assert!(may_feed(5_000, 0, false, DEADLINE, ALIVE));
        assert_eq!(feed_verdict(5_001, 0, false, DEADLINE, ALIVE), Feed::NoProgress);
    }

    #[test]
    fn the_deadline_wins_over_a_fresh_alive_call() {
        assert_eq!(feed_verdict(60_000, 60_000, false, DEADLINE, ALIVE), Feed::LifetimeOver);
        assert!(may_feed(59_999, 59_999, false, DEADLINE, ALIVE));
    }

    #[test]
    fn released_stops_the_feeding_whatever_else_holds() {
        for (now, at) in [(0, -1), (1_000, 1_000), (59_999, 59_999), (70_000, -1)] {
            assert_eq!(feed_verdict(now, at, true, DEADLINE, ALIVE), Feed::Released, "{now} {at}");
        }
    }

    #[test]
    fn a_deadline_of_zero_never_feeds() {
        // t6_watchdog_deadline_ms() is 0 in the proof build
        assert_eq!(feed_verdict(0, -1, false, 0, ALIVE), Feed::LifetimeOver);
    }
}
