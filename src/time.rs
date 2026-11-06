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

    fn as_nanos(&self) -> i128 {
        self.secs as i128 * 1_000_000_000 + self.nanos as i128
    }

    fn from_nanos(n: i128) -> Option<SystemTime> {
        let secs = n.div_euclid(1_000_000_000);
        let nanos = n.rem_euclid(1_000_000_000) as u32;
        Some(SystemTime { secs: i64::try_from(secs).ok()?, nanos })
    }

    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> {
        let diff = self.as_nanos() - earlier.as_nanos();
        let d = |n: i128| Duration::new((n / 1_000_000_000) as u64, (n % 1_000_000_000) as u32);
        if diff >= 0 { Ok(d(diff)) } else { Err(SystemTimeError(d(-diff))) }
    }

    pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
        SystemTime::now().duration_since(*self)
    }

    pub fn checked_add(&self, d: Duration) -> Option<SystemTime> {
        Self::from_nanos(self.as_nanos().checked_add(d.as_nanos() as i128)?)
    }

    pub fn checked_sub(&self, d: Duration) -> Option<SystemTime> {
        Self::from_nanos(self.as_nanos().checked_sub(d.as_nanos() as i128)?)
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

    /// The nanosecond representation used for SystemTime arithmetic
    /// round-trips for every representable time.
    #[kani::proof]
    fn system_time_nanos_roundtrip() {
        let t = SystemTime { secs: kani::any(), nanos: kani::any_where(|&n| n < 1_000_000_000) };
        assert!(SystemTime::from_nanos(t.as_nanos()) == Some(t));
    }

    /// duration_since succeeds exactly when the other time is not later.
    #[kani::proof]
    fn duration_since_sign() {
        let a = SystemTime { secs: kani::any_where(|s: &i64| s.unsigned_abs() < 1 << 40), nanos: kani::any_where(|&n| n < 1_000_000_000) };
        let b = SystemTime { secs: kani::any_where(|s: &i64| s.unsigned_abs() < 1 << 40), nanos: kani::any_where(|&n| n < 1_000_000_000) };
        assert_eq!(a.duration_since(b).is_ok(), a >= b);
    }
}
