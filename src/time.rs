//! Temporal quantification.

use core::error::Error;
use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};

pub use core::time::{Duration, TryFromFloatSecsError};

use crate::sys::linux::{self, CLOCK_MONOTONIC, CLOCK_REALTIME};

fn now(clock: usize) -> Duration {
    let ts = linux::clock_gettime(clock);
    Duration::new(ts.sec as u64, ts.nsec as u32)
}

/// A monotonic clock reading (`CLOCK_MONOTONIC`).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(Duration);

impl Instant {
    pub fn now() -> Instant {
        Instant(now(CLOCK_MONOTONIC))
    }

    pub fn duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier).unwrap_or_default()
    }

    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }

    pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier).unwrap_or_default()
    }

    pub fn elapsed(&self) -> Duration {
        Instant::now().duration_since(*self)
    }

    pub fn checked_add(&self, d: Duration) -> Option<Instant> {
        self.0.checked_add(d).map(Instant)
    }

    pub fn checked_sub(&self, d: Duration) -> Option<Instant> {
        self.0.checked_sub(d).map(Instant)
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, d: Duration) -> Instant {
        self.checked_add(d).expect("overflow when adding duration to instant")
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, d: Duration) {
        *self = *self + d;
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;
    fn sub(self, d: Duration) -> Instant {
        self.checked_sub(d).expect("overflow when subtracting duration from instant")
    }
}

impl SubAssign<Duration> for Instant {
    fn sub_assign(&mut self, d: Duration) {
        *self = *self - d;
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;
    fn sub(self, other: Instant) -> Duration {
        self.duration_since(other)
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instant").field("tv_sec", &self.0.as_secs()).field("tv_nsec", &self.0.subsec_nanos()).finish()
    }
}

/// A wall-clock reading (`CLOCK_REALTIME`), stored as an offset from the
/// Unix epoch (possibly negative).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemTime {
    // seconds and nanoseconds since the epoch, nanos always in 0..1e9
    secs: i64,
    nanos: u32,
}

pub const UNIX_EPOCH: SystemTime = SystemTime { secs: 0, nanos: 0 };

const NSEC: u32 = 1_000_000_000;

#[derive(Clone, Debug)]
pub struct SystemTimeError(Duration);

impl SystemTimeError {
    pub fn duration(&self) -> Duration {
        self.0
    }
}

impl fmt::Display for SystemTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("second time provided was later than self")
    }
}

impl Error for SystemTimeError {}

impl SystemTime {
    pub const UNIX_EPOCH: SystemTime = UNIX_EPOCH;

    pub fn now() -> SystemTime {
        let ts = linux::clock_gettime(CLOCK_REALTIME);
        SystemTime { secs: ts.sec, nanos: ts.nsec as u32 }
    }

    /// `self - earlier` for `self >= earlier` (the difference of two i64
    /// second counts always fits in a u64).
    fn sub_parts(&self, earlier: &SystemTime) -> Duration {
        if self.nanos >= earlier.nanos {
            Duration::new(self.secs.wrapping_sub(earlier.secs) as u64, self.nanos - earlier.nanos)
        } else {
            Duration::new(self.secs.wrapping_sub(earlier.secs).wrapping_sub(1) as u64, self.nanos + NSEC - earlier.nanos)
        }
    }

    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> {
        if *self >= earlier { Ok(self.sub_parts(&earlier)) } else { Err(SystemTimeError(earlier.sub_parts(self))) }
    }

    pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
        SystemTime::now().duration_since(*self)
    }

    // seconds/nanoseconds arithmetic with a carry, as std's Timespec does
    pub fn checked_add(&self, d: Duration) -> Option<SystemTime> {
        let mut secs = self.secs.checked_add_unsigned(d.as_secs())?;
        let mut nanos = self.nanos + d.subsec_nanos();
        if nanos >= NSEC {
            nanos -= NSEC;
            secs = secs.checked_add(1)?;
        }
        Some(SystemTime { secs, nanos })
    }

    pub fn checked_sub(&self, d: Duration) -> Option<SystemTime> {
        let mut secs = self.secs.checked_sub_unsigned(d.as_secs())?;
        let mut nanos = self.nanos as i32 - d.subsec_nanos() as i32;
        if nanos < 0 {
            nanos += NSEC as i32;
            secs = secs.checked_sub(1)?;
        }
        Some(SystemTime { secs, nanos: nanos as u32 })
    }

    pub(crate) fn from_parts(secs: i64, nanos: u32) -> SystemTime {
        SystemTime { secs, nanos }
    }
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime;
    fn add(self, d: Duration) -> SystemTime {
        self.checked_add(d).expect("overflow when adding duration to instant")
    }
}

impl AddAssign<Duration> for SystemTime {
    fn add_assign(&mut self, d: Duration) {
        *self = *self + d;
    }
}

impl Sub<Duration> for SystemTime {
    type Output = SystemTime;
    fn sub(self, d: Duration) -> SystemTime {
        self.checked_sub(d).expect("overflow when subtracting duration from instant")
    }
}

impl SubAssign<Duration> for SystemTime {
    fn sub_assign(&mut self, d: Duration) {
        *self = *self - d;
    }
}

impl fmt::Debug for SystemTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemTime").field("tv_sec", &self.secs).field("tv_nsec", &self.nanos).finish()
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    fn any_time() -> SystemTime {
        SystemTime { secs: kani::any(), nanos: kani::any_where(|&n| n < NSEC) }
    }

    fn any_duration() -> Duration {
        Duration::new(kani::any(), kani::any_where(|&n| n < NSEC))
    }

    /// duration_since succeeds exactly when the other time is not later.
    #[kani::proof]
    fn duration_since_matches_order() {
        let (a, b) = (any_time(), any_time());
        assert_eq!(a.duration_since(b).is_ok(), a >= b);
    }

    /// Adding a duration and measuring back recovers it, and subtracting it
    /// recovers the original time.
    #[kani::proof]
    fn add_then_measure_roundtrip() {
        let (t, d) = (any_time(), any_duration());
        if let Some(u) = t.checked_add(d) {
            assert!(u.duration_since(t).ok() == Some(d));
            assert!(u.checked_sub(d) == Some(t));
        }
    }

    /// The measured difference between two times takes one to the other.
    #[kani::proof]
    fn measure_then_add_roundtrip() {
        let (a, b) = (any_time(), any_time());
        match a.duration_since(b) {
            Ok(d) => assert!(b.checked_add(d) == Some(a)),
            Err(e) => assert!(a.checked_add(e.duration()) == Some(b)),
        }
    }
}
