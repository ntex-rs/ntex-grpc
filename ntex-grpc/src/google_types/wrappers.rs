#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::derivable_impls,
    clippy::unit_arg,
    clippy::derive_partial_eq_without_eq
)]
/// DO NOT MODIFY. Auto-generated file

// ///  Wrapper message for `double`.
// ///
// ///  The JSON representation for `DoubleValue` is JSON number.
// #[derive(Clone, PartialEq, Debug)]
// pub struct DoubleValue {
//     ///  The double value.
//     pub value: f64,
// }

// ///  Wrapper message for `float`.
// ///
// ///  The JSON representation for `FloatValue` is JSON number.
// #[derive(Clone, PartialEq, Debug)]
// pub struct FloatValue {
//     ///  The float value.
//     pub value: f32,
// }

///  Wrapper message for `int64`.
///
///  The JSON representation for `Int64Value` is JSON string.
#[derive(Clone, PartialEq, Debug)]
pub struct Int64Value {
    ///  The int64 value.
    pub value: i64,
}

///  Wrapper message for `uint64`.
///
///  The JSON representation for `UInt64Value` is JSON string.
#[derive(Clone, PartialEq, Debug)]
pub struct UInt64Value {
    ///  The uint64 value.
    pub value: u64,
}

///  Wrapper message for `int32`.
///
///  The JSON representation for `Int32Value` is JSON number.
#[derive(Clone, PartialEq, Debug)]
pub struct Int32Value {
    ///  The int32 value.
    pub value: i32,
}

///  Wrapper message for `uint32`.
///
///  The JSON representation for `UInt32Value` is JSON number.
#[derive(Clone, PartialEq, Debug)]
pub struct UInt32Value {
    ///  The uint32 value.
    pub value: u32,
}

///  Wrapper message for `bool`.
///
///  The JSON representation for `BoolValue` is JSON `true` and `false`.
#[derive(Clone, PartialEq, Debug)]
pub struct BoolValue {
    ///  The bool value.
    pub value: bool,
}

///  Wrapper message for `string`.
///
///  The JSON representation for `StringValue` is JSON string.
#[derive(Clone, PartialEq, Debug)]
pub struct StringValue {
    ///  The string value.
    pub value: crate::ByteString,
}

///  Wrapper message for `bytes`.
///
///  The JSON representation for `BytesValue` is JSON string.
#[derive(Clone, PartialEq, Debug)]
pub struct BytesValue {
    ///  The bytes value.
    pub value: crate::Bytes,
}

mod _priv_impl {
    use super::*;

    // impl crate::Message for DoubleValue {
    //     #[inline]
    //     fn write(&self, dst: &mut crate::BytesMut) {
    //         crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
    //     }

    //     #[inline]
    //     fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
    //         const STRUCT_NAME: &str = "DoubleValue";
    //         let mut msg = Self::default();
    //         while !src.is_empty() {
    //             let (tag, wire_type) = crate::encoding::decode_key(src)?;
    //             match tag {
    //                 1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
    //                     .map_err(|err| err.push(STRUCT_NAME, "value"))?,
    //                 _ => crate::encoding::skip_field(wire_type, tag, src)?,
    //             }
    //         }
    //         Ok(msg)
    //     }

    //     #[inline]
    //     fn encoded_len(&self) -> usize {
    //         0 + crate::NativeType::serialized_len(
    //             &self.value,
    //             1,
    //             crate::types::DefaultValue::Default,
    //         )
    //     }
    // }

    // impl ::std::default::Default for DoubleValue {
    //     #[inline]
    //     fn default() -> Self {
    //         Self {
    //             value: ::core::default::Default::default(),
    //         }
    //     }
    // }

    // impl crate::Message for FloatValue {
    //     #[inline]
    //     fn write(&self, dst: &mut crate::BytesMut) {
    //         crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
    //     }

    //     #[inline]
    //     fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
    //         const STRUCT_NAME: &str = "FloatValue";
    //         let mut msg = Self::default();
    //         while !src.is_empty() {
    //             let (tag, wire_type) = crate::encoding::decode_key(src)?;
    //             match tag {
    //                 1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
    //                     .map_err(|err| err.push(STRUCT_NAME, "value"))?,
    //                 _ => crate::encoding::skip_field(wire_type, tag, src)?,
    //             }
    //         }
    //         Ok(msg)
    //     }

    //     #[inline]
    //     fn encoded_len(&self) -> usize {
    //         0 + crate::NativeType::serialized_len(
    //             &self.value,
    //             1,
    //             crate::types::DefaultValue::Default,
    //         )
    //     }
    // }

    // impl ::std::default::Default for FloatValue {
    //     #[inline]
    //     fn default() -> Self {
    //         Self {
    //             value: ::core::default::Default::default(),
    //         }
    //     }
    // }

    impl crate::Message for Int64Value {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "Int64Value";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Int64Value {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }

    impl crate::Message for UInt64Value {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "UInt64Value";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for UInt64Value {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }

    impl crate::Message for Int32Value {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "Int32Value";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Int32Value {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }

    impl crate::Message for UInt32Value {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "UInt32Value";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for UInt32Value {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }

    impl crate::Message for BoolValue {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "BoolValue";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for BoolValue {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }

    impl crate::Message for StringValue {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "StringValue";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for StringValue {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }

    impl crate::Message for BytesValue {
        #[inline]
        fn write(&self, dst: &mut crate::BytesMut) {
            crate::NativeType::serialize(&self.value, 1, crate::types::DefaultValue::Default, dst);
        }

        #[inline]
        fn read(src: &mut crate::Bytes) -> ::std::result::Result<Self, crate::DecodeError> {
            const STRUCT_NAME: &str = "BytesValue";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = crate::encoding::decode_key(src)?;
                match tag {
                    1 => crate::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    _ => crate::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + crate::NativeType::serialized_len(
                &self.value,
                1,
                crate::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for BytesValue {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
            }
        }
    }
}
