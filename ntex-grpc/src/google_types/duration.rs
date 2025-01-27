#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::derivable_impls,
    clippy::unit_arg,
    clippy::derive_partial_eq_without_eq
)]
//! DO NOT MODIFY. Auto-generated file

///  A Duration represents a signed, fixed-length span of time represented
///  as a count of seconds and fractions of seconds at nanosecond
///  resolution. It is independent of any calendar and concepts like "day"
///  or "month". It is related to Timestamp in that the difference between
///  two Timestamp values is a Duration and it can be added or subtracted
///  from a Timestamp. Range is approximately +-10,000 years.
///
///  # Examples
///
///  Example 1: Compute Duration from two Timestamps in pseudo code.
///
///      Timestamp start = ...;
///      Timestamp end = ...;
///      Duration duration = ...;
///
///      duration.seconds = end.seconds - start.seconds;
///      duration.nanos = end.nanos - start.nanos;
///
///      if (duration.seconds < 0 && duration.nanos > 0) {
///        duration.seconds += 1;
///        duration.nanos -= 1000000000;
///      } else if (duration.seconds > 0 && duration.nanos < 0) {
///        duration.seconds -= 1;
///        duration.nanos += 1000000000;
///      }
///
///  Example 2: Compute Timestamp from Timestamp + Duration in pseudo code.
///
///      Timestamp start = ...;
///      Duration duration = ...;
///      Timestamp end = ...;
///
///      end.seconds = start.seconds + duration.seconds;
///      end.nanos = start.nanos + duration.nanos;
///
///      if (end.nanos < 0) {
///        end.seconds -= 1;
///        end.nanos += 1000000000;
///      } else if (end.nanos >= 1000000000) {
///        end.seconds += 1;
///        end.nanos -= 1000000000;
///      }
///
///  Example 3: Compute Duration from datetime.timedelta in Python.
///
///      td = datetime.timedelta(days=3, minutes=10)
///      duration = Duration()
///      duration.FromTimedelta(td)
///
///  # JSON Mapping
///
///  In JSON format, the Duration type is encoded as a string rather than an
///  object, where the string ends in the suffix "s" (indicating seconds) and
///  is preceded by the number of seconds, with nanoseconds expressed as
///  fractional seconds. For example, 3 seconds with 0 nanoseconds should be
///  encoded in JSON format as "3s", while 3 seconds and 1 nanosecond should
///  be expressed in JSON format as "3.000000001s", and 3 seconds and 1
///  microsecond should be expressed in JSON format as "3.000001s".
///
///
#[derive(Clone, PartialEq, Debug)]
pub struct Duration {
    ///  Signed seconds of the span of time. Must be from -315,576,000,000
    ///  to +315,576,000,000 inclusive. Note: these bounds are computed from:
    ///  60 sec/min * 60 min/hr * 24 hr/day * 365.25 days/year * 10000 years
    pub seconds: i64,
    ///  Signed fractions of a second at nanosecond resolution of the span
    ///  of time. Durations less than one second are represented with a 0
    ///  `seconds` field and a positive or negative `nanos` field. For durations
    ///  of one second or more, a non-zero value for the `nanos` field must be
    ///  of the same sign as the `seconds` field. Must be from -999,999,999
    ///  to +999,999,999 inclusive.
    pub nanos: i32,
}

mod _priv_impl {
    use super::*;

    impl crate::Message for Duration {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(
                &self.seconds,
                1,
                crate::types::DefaultValue::Default,
                dst,
            );
            crate::NativeType::serialize(&self.nanos, 2, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "Duration";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.seconds, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "seconds"))?,
                    2 => crate::NativeType::deserialize(&mut msg.nanos, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "nanos"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.seconds,
                1,
                crate::types::DefaultValue::Default,
            ) + crate::NativeType::serialized_len(
                &self.nanos,
                2,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Duration {
        #[inline]
        fn default() -> Self {
            Self {
                seconds: ::core::default::Default::default(),
                nanos: ::core::default::Default::default(),
            }
        }
    }
}
