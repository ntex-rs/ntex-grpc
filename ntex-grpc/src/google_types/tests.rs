use std::convert::TryFrom;
use std::time::{self, SystemTime, UNIX_EPOCH};

use ntex_bytes::{ByteString, Bytes};

use super::*;
use crate::{BytePages, DecodeError, Message};

/// Writes `msg` and checks that `encoded_len()` agrees with the number of
/// bytes `write()` produced.
fn encode<T: Message>(msg: &T) -> Bytes {
    let mut pages = BytePages::default();
    msg.write(&mut pages);
    assert_eq!(
        msg.encoded_len(),
        pages.len(),
        "encoded_len() does not match the written length of {msg:?}"
    );
    pages.freeze()
}

fn decode<T: Message>(bytes: &[u8]) -> Result<T, DecodeError> {
    T::read(&mut Bytes::copy_from_slice(bytes))
}

/// `write()` must produce exactly `expected`, and `read()` must give `msg` back.
fn check<T: Message + PartialEq>(msg: &T, expected: &[u8]) {
    assert_eq!(&encode(msg)[..], expected, "wire bytes of {msg:?}");
    assert_eq!(&decode::<T>(expected).unwrap(), msg);
}

/// Unknown fields of every skippable wire type, placed before the known ones.
const UNKNOWN_FIELDS: &[u8] = &[
    0x38, 0x7f, // 7: varint
    0x41, 1, 2, 3, 4, 5, 6, 7, 8, // 8: 64-bit
    0x4a, 0x02, 0xaa, 0xbb, // 9: length delimited
    0x55, 1, 2, 3, 4, // 10: 32-bit
];

#[test]
fn duration_wire_format() {
    check(&Duration::default(), &[]);
    check(
        &Duration {
            seconds: 3,
            nanos: 1,
        },
        &[0x08, 0x03, 0x10, 0x01],
    );
    // fields holding the default value are left out
    check(
        &Duration {
            seconds: 5,
            nanos: 0,
        },
        &[0x08, 0x05],
    );
    check(
        &Duration {
            seconds: 0,
            nanos: 7,
        },
        &[0x10, 0x07],
    );
    // int64/int32 are sign extended to 64 bits before the varint encoding
    check(
        &Duration {
            seconds: -1,
            nanos: -500_000_000,
        },
        &[
            0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0x10, 0x80, 0xb6,
            0xca, 0x91, 0xfe, 0xff, 0xff, 0xff, 0xff, 0x01,
        ],
    );
}

#[test]
fn duration_read_skips_unknown_fields() {
    let mut src = vec![0x08, 0x02];
    src.extend_from_slice(UNKNOWN_FIELDS);
    src.extend_from_slice(&[0x10, 0x09]);
    assert_eq!(
        decode::<Duration>(&src).unwrap(),
        Duration {
            seconds: 2,
            nanos: 9
        }
    );
}

#[test]
fn duration_read_errors() {
    // `seconds` key without a value
    assert!(decode::<Duration>(&[0x08]).is_err());
    // `nanos` key without a value
    assert!(decode::<Duration>(&[0x08, 0x01, 0x10]).is_err());
    // unterminated varint
    assert!(decode::<Duration>(&[0x08, 0xff, 0xff]).is_err());
    // wrong wire type for `seconds`, the field name is in the error
    let err = decode::<Duration>(&[0x0a, 0x01, 0x00]).unwrap_err();
    assert!(err.to_string().contains("Duration.seconds"), "{err}");
    // wrong wire type for `nanos`
    let err = decode::<Duration>(&[0x12, 0x01, 0x00]).unwrap_err();
    assert!(err.to_string().contains("Duration.nanos"), "{err}");
    // unknown field claiming more bytes than are left
    assert!(decode::<Duration>(&[0x4a, 0x10, 0x00]).is_err());
    // tag 0 is not a valid tag
    assert!(decode::<Duration>(&[0x00, 0x00]).is_err());
}

#[test]
fn duration_normalize() {
    fn norm(seconds: i64, nanos: i32) -> (i64, i32) {
        let mut d = Duration { seconds, nanos };
        d.normalize();
        (d.seconds, d.nanos)
    }

    // already canonical
    assert_eq!(norm(0, 0), (0, 0));
    assert_eq!(norm(1, 2), (1, 2));
    assert_eq!(norm(0, 999_999_999), (0, 999_999_999));
    assert_eq!(norm(0, -999_999_999), (0, -999_999_999));

    // whole seconds move out of `nanos`
    assert_eq!(norm(0, 1_000_000_000), (1, 0));
    assert_eq!(norm(0, 1_500_000_000), (1, 500_000_000));
    assert_eq!(norm(0, -1_500_000_000), (-1, -500_000_000));
    assert_eq!(norm(3, -2_000_000_000), (1, 0));

    // `nanos` takes the sign of `seconds`
    assert_eq!(norm(1, -500_000_000), (0, 500_000_000));
    assert_eq!(norm(-1, 500_000_000), (0, -500_000_000));
    assert_eq!(norm(-5, 1_500_000_000), (-3, -500_000_000));

    // overflow clamps to the largest/smallest duration
    assert_eq!(norm(i64::MAX, 1_500_000_000), (i64::MAX, 999_999_999));
    assert_eq!(norm(i64::MIN, -1_500_000_000), (i64::MIN, -999_999_999));
}

#[test]
fn duration_from_std() {
    assert_eq!(
        Duration::try_from(time::Duration::new(3, 500_000_000)).unwrap(),
        Duration {
            seconds: 3,
            nanos: 500_000_000
        }
    );
    assert_eq!(
        Duration::try_from(time::Duration::ZERO).unwrap(),
        Duration::default()
    );
    assert_eq!(
        Duration::try_from(time::Duration::new(i64::MAX as u64, 999_999_999)).unwrap(),
        Duration {
            seconds: i64::MAX,
            nanos: 999_999_999
        }
    );

    // more than i64::MAX seconds does not fit
    let err = Duration::try_from(time::Duration::new(u64::MAX, 0)).unwrap_err();
    assert_eq!(err.to_string(), "Duration is out of range");
    assert!(format!("{err:?}").contains("OutOfRangeDurationError"));
    let err: Box<dyn std::error::Error> = Box::new(OutOfRangeDurationError);
    assert!(err.source().is_none());
}

#[test]
fn duration_into_std() {
    assert_eq!(
        time::Duration::try_from(Duration {
            seconds: 3,
            nanos: 500_000_000
        })
        .unwrap(),
        time::Duration::new(3, 500_000_000)
    );
    // the value is normalized first
    assert_eq!(
        time::Duration::try_from(Duration {
            seconds: 0,
            nanos: 1_500_000_000
        })
        .unwrap(),
        time::Duration::new(1, 500_000_000)
    );

    let err = time::Duration::try_from(Duration {
        seconds: -1,
        nanos: -500_000_000,
    })
    .unwrap_err();
    assert_eq!(err.0, time::Duration::new(1, 500_000_000));
    assert_eq!(err.to_string(), "Duration is negative: -1.5s");
    assert!(format!("{err:?}").contains("NegativeDurationError"));
    let err: Box<dyn std::error::Error> = Box::new(err);
    assert!(err.source().is_none());
}

#[test]
fn duration_subsecond_negative_into_std_wraps_around() {
    // BUG: a negative duration smaller than a second keeps `seconds == 0` after
    // normalize(), so the conversion takes the non-negative branch and the
    // negative `nanos` wrap around into a large positive value instead of
    // producing a NegativeDurationError.
    assert_eq!(
        time::Duration::try_from(Duration {
            seconds: 0,
            nanos: -1
        })
        .unwrap(),
        time::Duration::new(4, 294_967_295)
    );
}

#[test]
fn timestamp_wire_format() {
    check(&Timestamp::default(), &[]);
    check(
        &Timestamp {
            seconds: 1_700_000_000,
            nanos: 123_456_789,
        },
        &[
            0x08, 0x80, 0xe2, 0xcf, 0xaa, 0x06, 0x10, 0x95, 0x9a, 0xef, 0x3a,
        ],
    );
    check(
        &Timestamp {
            seconds: 0,
            nanos: 1,
        },
        &[0x10, 0x01],
    );
    check(
        &Timestamp {
            seconds: -1,
            nanos: 0,
        },
        &[
            0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        ],
    );
    // 0001-01-01T00:00:00Z, the lower bound of the type
    check(
        &Timestamp {
            seconds: -62_135_596_800,
            nanos: 0,
        },
        &[
            0x08, 0x80, 0x92, 0xb8, 0xc3, 0x98, 0xfe, 0xff, 0xff, 0xff, 0x01,
        ],
    );
}

#[test]
fn timestamp_read_skips_unknown_fields() {
    let mut src = vec![0x08, 0x02];
    src.extend_from_slice(UNKNOWN_FIELDS);
    src.extend_from_slice(&[0x10, 0x09]);
    assert_eq!(
        decode::<Timestamp>(&src).unwrap(),
        Timestamp {
            seconds: 2,
            nanos: 9
        }
    );
}

#[test]
fn timestamp_read_errors() {
    assert!(decode::<Timestamp>(&[0x08]).is_err());
    assert!(decode::<Timestamp>(&[0x08, 0x01, 0x10]).is_err());
    let err = decode::<Timestamp>(&[0x0a, 0x01, 0x00]).unwrap_err();
    assert!(err.to_string().contains("Timestamp.seconds"), "{err}");
    let err = decode::<Timestamp>(&[0x12, 0x01, 0x00]).unwrap_err();
    assert!(err.to_string().contains("Timestamp.nanos"), "{err}");
}

#[test]
fn timestamp_now() {
    let ts = Timestamp::now();
    // somewhere after 2020-01-01 and with `nanos` in range
    assert!(ts.seconds > 1_577_836_800, "{}", ts.seconds);
    assert!((0..1_000_000_000).contains(&ts.nanos), "{}", ts.nanos);

    let elapsed = SystemTime::try_from(ts.clone())
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    assert_eq!(elapsed.as_secs(), ts.seconds as u64);
    assert_eq!(elapsed.subsec_nanos(), ts.nanos as u32);
}

#[test]
fn timestamp_into_system_time() {
    assert_eq!(
        SystemTime::try_from(Timestamp::default()).unwrap(),
        UNIX_EPOCH
    );
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: 1,
            nanos: 500_000_000
        })
        .unwrap(),
        UNIX_EPOCH + time::Duration::new(1, 500_000_000)
    );
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: -2,
            nanos: 0
        })
        .unwrap(),
        UNIX_EPOCH - time::Duration::from_secs(2)
    );
    // whole seconds are carried out of `nanos`
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: 0,
            nanos: 1_500_000_000
        })
        .unwrap(),
        UNIX_EPOCH + time::Duration::new(1, 500_000_000)
    );
    // negative `nanos` borrow a second, as the type requires nanos >= 0
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: 0,
            nanos: -1
        })
        .unwrap(),
        UNIX_EPOCH - time::Duration::from_nanos(1)
    );
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: 0,
            nanos: -1_500_000_000
        })
        .unwrap(),
        UNIX_EPOCH - time::Duration::new(1, 500_000_000)
    );

    // carrying whole seconds out of `nanos` overflows `seconds`, so the value
    // is clamped to the latest representable timestamp
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: i64::MAX,
            nanos: 1_500_000_000
        }),
        SystemTime::try_from(Timestamp {
            seconds: i64::MAX,
            nanos: 999_999_999
        })
    );
}

#[test]
fn timestamp_into_system_time_out_of_range() {
    // i64::MIN seconds cannot be negated
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: i64::MIN,
            nanos: 0
        })
        .unwrap_err(),
        "time value is out of supported range"
    );
    // borrowing a second underflows, normalize() clamps `nanos` to 0
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: i64::MIN,
            nanos: -1
        })
        .unwrap_err(),
        "time value is out of supported range"
    );
    // carrying seconds out of `nanos` underflows
    assert_eq!(
        SystemTime::try_from(Timestamp {
            seconds: i64::MIN,
            nanos: -1_500_000_000
        })
        .unwrap_err(),
        "time value is out of supported range"
    );
}

/// Exercises the generated `Message` impl of a wrapper type: exact wire bytes,
/// `encoded_len()`, round-trip, default skipping, unknown field skipping and
/// truncated input.
macro_rules! check_wrapper {
    ($ty:ident, $($value:expr => $bytes:expr),+ $(,)?) => {{
        let def = $ty::default();
        assert_eq!(def.encoded_len(), 0, "{} default must be skipped", stringify!($ty));
        assert!(encode(&def).is_empty());
        assert_eq!(decode::<$ty>(&[]).unwrap(), def);
        $({
            let bytes: &[u8] = &$bytes;
            let msg = $ty { value: $value };
            assert_ne!(msg, def);
            check(&msg, bytes);

            let mut src = UNKNOWN_FIELDS.to_vec();
            src.extend_from_slice(bytes);
            assert_eq!(decode::<$ty>(&src).unwrap(), msg);

            assert!(
                decode::<$ty>(&bytes[..bytes.len() - 1]).is_err(),
                "{} must reject truncated input", stringify!($ty)
            );
        })+
    }};
}

#[test]
fn wrapper_wire_format() {
    check_wrapper!(
        DoubleValue,
        1.5f64 => [0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0x3f],
        -1.5f64 => [0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0xbf],
    );
    check_wrapper!(
        Int64Value,
        1i64 => [0x08, 0x01],
        i64::MAX => [0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
        i64::MIN => [0x08, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01],
    );
    check_wrapper!(
        UInt64Value,
        300u64 => [0x08, 0xac, 0x02],
        u64::MAX => [0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
    );
    check_wrapper!(
        Int32Value,
        300i32 => [0x08, 0xac, 0x02],
        // negative int32 is sign extended to 64 bits, so it takes 10 bytes
        (-2i32) => [0x08, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
    );
    check_wrapper!(
        UInt32Value,
        300u32 => [0x08, 0xac, 0x02],
        u32::MAX => [0x08, 0xff, 0xff, 0xff, 0xff, 0x0f],
    );
    check_wrapper!(BoolValue, true => [0x08, 0x01]);
    check_wrapper!(
        StringValue,
        ByteString::from("hello") => [0x0a, 0x05, b'h', b'e', b'l', b'l', b'o'],
    );
    check_wrapper!(
        BytesValue,
        Bytes::from_static(&[1, 2, 3]) => [0x0a, 0x03, 0x01, 0x02, 0x03],
    );
}

#[test]
fn string_value_rejects_invalid_utf8() {
    let err = decode::<StringValue>(&[0x0a, 0x01, 0xff]).unwrap_err();
    assert!(err.to_string().contains("StringValue.value"), "{err}");
    assert!(err.to_string().contains("not UTF-8"), "{err}");
}

#[test]
fn float_value_writes_a_bogus_length_prefix() {
    // BUG: an `f32` field is written as `key + varint(4) + 4 bytes` instead of
    // the fixed32 form `key + 4 bytes`, while `encoded_len()` reports the
    // correct 5 bytes. So `encoded_len()` disagrees with `write()` and the
    // bytes are not readable by a conforming protobuf implementation.
    let msg = FloatValue { value: 1.5 };
    let mut pages = BytePages::default();
    msg.write(&mut pages);
    let buf = pages.freeze();

    assert_eq!(&buf[..], [0x0d, 0x04, 0x00, 0x00, 0xc0, 0x3f].as_slice());
    assert_eq!(msg.encoded_len(), 5);
    assert_eq!(buf.len(), 6);

    // it still round-trips within this crate
    assert_eq!(FloatValue::read(&mut buf.clone()).unwrap(), msg);
    // ... but the spec conformant encoding is rejected
    assert!(decode::<FloatValue>(&[0x0d, 0x00, 0x00, 0xc0, 0x3f]).is_err());

    let def = FloatValue::default();
    assert_eq!(def.encoded_len(), 0);
    assert_eq!(decode::<FloatValue>(&[]).unwrap(), def);

    let mut src = UNKNOWN_FIELDS.to_vec();
    src.extend_from_slice(&buf[..]);
    assert_eq!(decode::<FloatValue>(&src).unwrap(), msg);
    assert!(decode::<FloatValue>(&buf[..buf.len() - 1]).is_err());
}
