use std::{convert::TryFrom, time};

use super::Duration;

const NANOS_PER_SECOND: i32 = 1_000_000_000;
const NANOS_MAX: i32 = NANOS_PER_SECOND - 1;

impl Duration {
    /// Bring the value into its canonical form.
    ///
    /// Whole seconds move out of `nanos`, and `nanos` gets the same sign as
    /// `seconds`. A value that doesn't fit is clamped to the largest or
    /// smallest duration.
    pub fn normalize(&mut self) {
        // Make sure nanos is in the range.
        if self.nanos <= -NANOS_PER_SECOND || self.nanos >= NANOS_PER_SECOND {
            if let Some(seconds) = self
                .seconds
                .checked_add((self.nanos / NANOS_PER_SECOND) as i64)
            {
                self.seconds = seconds;
                self.nanos %= NANOS_PER_SECOND;
            } else if self.nanos < 0 {
                // Negative overflow! Set to the least normal value.
                self.seconds = i64::MIN;
                self.nanos = -NANOS_MAX;
            } else {
                // Positive overflow! Set to the greatest normal value.
                self.seconds = i64::MAX;
                self.nanos = NANOS_MAX;
            }
        }

        // nanos should have the same sign as seconds.
        if self.seconds < 0 && self.nanos > 0 {
            if let Some(seconds) = self.seconds.checked_add(1) {
                self.seconds = seconds;
                self.nanos -= NANOS_PER_SECOND;
            } else {
                // Positive overflow! Set to the greatest normal value.
                debug_assert_eq!(self.seconds, i64::MAX);
                self.nanos = NANOS_MAX;
            }
        } else if self.seconds > 0 && self.nanos < 0 {
            if let Some(seconds) = self.seconds.checked_sub(1) {
                self.seconds = seconds;
                self.nanos += NANOS_PER_SECOND;
            } else {
                // Negative overflow! Set to the least normal value.
                debug_assert_eq!(self.seconds, i64::MIN);
                self.nanos = -NANOS_MAX;
            }
        }
    }
}

impl TryFrom<time::Duration> for Duration {
    type Error = OutOfRangeDurationError;

    /// Converts a `std::time::Duration` to a `Duration`, failing if the duration is too large.
    fn try_from(duration: time::Duration) -> Result<Duration, OutOfRangeDurationError> {
        let seconds = i64::try_from(duration.as_secs()).map_err(|_| OutOfRangeDurationError)?;
        let nanos = duration.subsec_nanos() as i32;

        let mut duration = Duration { seconds, nanos };
        duration.normalize();
        Ok(duration)
    }
}

impl TryFrom<Duration> for time::Duration {
    type Error = NegativeDurationError;

    /// Converts a `Duration` to a `std::time::Duration`, failing if the duration is negative.
    fn try_from(mut duration: Duration) -> Result<time::Duration, NegativeDurationError> {
        duration.normalize();
        // after normalize() nanos has the sign of seconds, or any sign if
        // seconds is zero
        let magnitude = time::Duration::new(
            duration.seconds.unsigned_abs(),
            duration.nanos.unsigned_abs(),
        );
        if duration.seconds < 0 || duration.nanos < 0 {
            Err(NegativeDurationError(magnitude))
        } else {
            Ok(magnitude)
        }
    }
}

/// Indicates failure to convert a Duration to a `std::time::Duration` because
/// the duration is negative. The included `std::time::Duration` matches the magnitude of the
/// original negative Duration.
#[derive(Debug)]
pub struct NegativeDurationError(pub time::Duration);

/// Indicates failure to convert a `std::time::Duration` to a Duration.
///
/// Converting a `std::time::Duration` to a Duration fails if the magnitude
/// exceeds that representable by Duration.
#[derive(Debug)]
pub struct OutOfRangeDurationError;

impl std::fmt::Display for NegativeDurationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Duration is negative: -{:?}", self.0)
    }
}

impl std::error::Error for NegativeDurationError {}

impl std::fmt::Display for OutOfRangeDurationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Duration is out of range")
    }
}

impl std::error::Error for OutOfRangeDurationError {}
