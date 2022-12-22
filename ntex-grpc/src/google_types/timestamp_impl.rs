use std::convert::TryFrom;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Timestamp;

impl Timestamp {
    pub fn now() -> Self {
        let dt = ntex_util::time::system_time()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        Timestamp {
            seconds: dt.as_secs() as i64,
            nanos: dt.subsec_nanos() as i32,
        }
    }

    fn normalize(&mut self) {
        const NANOS_PER_SECOND: i32 = 1_000_000_000;

        // Make sure nanos is in the range.
        if self.nanos <= -NANOS_PER_SECOND || self.nanos >= NANOS_PER_SECOND {
            if let Some(seconds) = self
                .seconds
                .checked_add((self.nanos / NANOS_PER_SECOND) as i64)
            {
                self.seconds = seconds;
                self.nanos %= NANOS_PER_SECOND;
            } else if self.nanos < 0 {
                // Negative overflow! Set to the earliest normal value.
                self.seconds = i64::MIN;
                self.nanos = 0;
            } else {
                // Positive overflow! Set to the latest normal value.
                self.seconds = i64::MAX;
                self.nanos = 999_999_999;
            }
        }

        // For Timestamp nanos should be in the range [0, 999999999].
        if self.nanos < 0 {
            if let Some(seconds) = self.seconds.checked_sub(1) {
                self.seconds = seconds;
                self.nanos += NANOS_PER_SECOND;
            } else {
                // Negative overflow! Set to the earliest normal value.
                debug_assert_eq!(self.seconds, i64::MIN);
                self.nanos = 0;
            }
        }
    }
}

impl TryFrom<Timestamp> for SystemTime {
    type Error = &'static str;

    fn try_from(mut timestamp: Timestamp) -> Result<SystemTime, Self::Error> {
        timestamp.normalize();

        let system_time = if timestamp.seconds >= 0 {
            UNIX_EPOCH.checked_add(Duration::from_secs(timestamp.seconds as u64))
        } else {
            UNIX_EPOCH.checked_sub(Duration::from_secs(
                timestamp
                    .seconds
                    .checked_neg()
                    .ok_or("time value is out of supported range")? as u64,
            ))
        };

        let system_time = system_time.and_then(|system_time| {
            system_time.checked_add(Duration::from_nanos(timestamp.nanos as u64))
        });

        system_time.ok_or("value is out of supported range")
    }
}
