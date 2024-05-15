#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::derivable_impls,
    clippy::unit_arg,
    clippy::derive_partial_eq_without_eq,
    clippy::manual_range_patterns
)]
/// DO NOT MODIFY. Auto-generated file

#[derive(Clone, PartialEq, Debug)]
pub struct SearchRequest {
    pub query: ::ntex_grpc::ByteString,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SearchResponse {
    pub results: Vec<Counts>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Count {
    pub value: f64,
    pub offset: u64,
    pub count: u64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Counts {
    pub counts: Vec<Count>,
}

/// `CountsSearch` service definition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountsSearch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountsSearchMethods {
    Search(CountsSearchSearchMethod),
}

#[derive(Debug, Clone)]
pub struct CountsSearchClient<T>(T);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CountsSearchSearchMethod;

impl ::ntex_grpc::MethodDef for CountsSearchSearchMethod {
    const NAME: &'static str = "Search";
    const PATH: ::ntex_grpc::ByteString =
        ::ntex_grpc::ByteString::from_static("/counts.CountsSearch/Search");
    type Input = SearchRequest;
    type Output = SearchResponse;
}

mod _priv_impl {
    use super::*;

    impl ::ntex_grpc::Message for SearchRequest {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.query,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "SearchRequest";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(&mut msg.query, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "query"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.query,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for SearchRequest {
        #[inline]
        fn default() -> Self {
            Self {
                query: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for SearchResponse {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.results,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "SearchResponse";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.results, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "results"))?
                    }
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.results,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for SearchResponse {
        #[inline]
        fn default() -> Self {
            Self {
                results: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for Count {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.value,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.offset,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.count,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "Count";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(&mut msg.value, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "value"))?,
                    2 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.offset, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "offset"))?
                    }
                    3 => ::ntex_grpc::NativeType::deserialize(&mut msg.count, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "count"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.value,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.offset,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.count,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Count {
        #[inline]
        fn default() -> Self {
            Self {
                value: ::core::default::Default::default(),
                offset: ::core::default::Default::default(),
                count: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for Counts {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytesMut) {
            ::ntex_grpc::NativeType::serialize(
                &self.counts,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "Counts";
            let mut msg = Self::default();
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => {
                        ::ntex_grpc::NativeType::deserialize(&mut msg.counts, tag, wire_type, src)
                            .map_err(|err| err.push(STRUCT_NAME, "counts"))?
                    }
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(msg)
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.counts,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Counts {
        #[inline]
        fn default() -> Self {
            Self {
                counts: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::ServiceDef for CountsSearch {
        const NAME: &'static str = "counts.CountsSearch";
        type Methods = CountsSearchMethods;

        #[inline]
        fn method_by_name(name: &str) -> Option<Self::Methods> {
            use ::ntex_grpc::MethodDef;
            match name {
                CountsSearchSearchMethod::NAME => {
                    Some(CountsSearchMethods::Search(CountsSearchSearchMethod))
                }
                _ => None,
            }
        }
    }

    impl<T> CountsSearchClient<T> {
        #[inline]
        /// Create new client instance
        pub fn new(transport: T) -> Self {
            Self(transport)
        }
    }

    impl<T> ::ntex_grpc::client::ClientInformation<T> for CountsSearchClient<T> {
        #[inline]
        /// Create new client instance
        fn create(transport: T) -> Self {
            Self(transport)
        }

        #[inline]
        /// Get referece to underlying transport
        fn transport(&self) -> &T {
            &self.0
        }

        #[inline]
        /// Get mut referece to underlying transport
        fn transport_mut(&mut self) -> &mut T {
            &mut self.0
        }

        #[inline]
        /// Consume client and return inner transport
        fn into_inner(self) -> T {
            self.0
        }
    }

    impl<T: ::ntex_grpc::client::Transport<CountsSearchSearchMethod>> CountsSearchClient<T> {
        pub fn search<'a>(
            &'a self,
            req: &'a super::SearchRequest,
        ) -> ::ntex_grpc::client::Request<'a, T, CountsSearchSearchMethod> {
            ::ntex_grpc::client::Request::new(&self.0, req)
        }
    }
}
