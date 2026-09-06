//! `hyper::rt::Timer` over geario's timer wheel.
//!
//! hyper needs a timer for anything with a deadline: HTTP/2 keep-alive
//! intervals and timeouts, HTTP/1 header read timeouts, connection idle
//! limits. Without one those settings are silently inert, so this is part of
//! matching hyper's feature set rather than an extra.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use geario::util::time::{Millis, Sleep as GearioSleep};
use hyper::rt::{Sleep, Timer};

/// A `hyper::rt::Timer` backed by geario's timer wheel.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct GearioTimer;

impl GearioTimer {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Timer for GearioTimer {
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Sleep>> {
        Box::pin(TimerSleep::new(duration))
    }

    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Sleep>> {
        Box::pin(TimerSleep::new(
            deadline.saturating_duration_since(Instant::now()),
        ))
    }

    fn reset(&self, sleep: &mut Pin<Box<dyn Sleep>>, new_deadline: Instant) {
        // Reuse the existing timer entry when this is one of ours; hyper
        // resets on every read for a keep-alive interval, and allocating a
        // fresh entry each time is what `reset` exists to avoid.
        if let Some(sleep) = sleep.as_mut().downcast_mut_pin::<TimerSleep>() {
            sleep.reset(new_deadline);
        } else {
            *sleep = self.sleep_until(new_deadline);
        }
    }
}

/// geario's timer wheel has millisecond resolution, so a sub-millisecond
/// deadline rounds up to one millisecond rather than firing immediately.
fn to_millis(duration: Duration) -> Millis {
    Millis(u32::try_from(duration.as_millis().max(1)).unwrap_or(u32::MAX))
}

#[derive(Debug)]
struct TimerSleep(GearioSleep);

impl TimerSleep {
    fn new(duration: Duration) -> Self {
        Self(GearioSleep::new(to_millis(duration)))
    }

    fn reset(self: Pin<&mut Self>, deadline: Instant) {
        self.0
            .reset(to_millis(deadline.saturating_duration_since(Instant::now())));
    }
}

impl Future for TimerSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0.poll_elapsed(cx)
    }
}

impl Sleep for TimerSleep {}

#[cfg(test)]
mod tests {
    use super::*;

    #[geario::test]
    async fn sleeps_for_about_the_requested_time() {
        let started = Instant::now();
        GearioTimer::new().sleep(Duration::from_millis(60)).await;
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(50), "returned early: {waited:?}");
        assert!(waited < Duration::from_secs(5), "far too long: {waited:?}");
    }

    /// A deadline below the wheel's resolution has to still be a deadline.
    /// Rounding it to zero would turn a timeout into a busy loop.
    #[geario::test]
    async fn a_sub_millisecond_deadline_still_elapses() {
        GearioTimer::new().sleep(Duration::from_nanos(1)).await;
    }

    /// Reset has to move the deadline of the existing timer, not be ignored.
    #[geario::test]
    async fn reset_extends_a_pending_sleep() {
        let timer = GearioTimer::new();
        let mut sleep = timer.sleep(Duration::from_millis(20));
        let started = Instant::now();
        timer.reset(&mut sleep, Instant::now() + Duration::from_millis(120));
        sleep.await;
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(100), "reset was ignored: {waited:?}");
    }
}
