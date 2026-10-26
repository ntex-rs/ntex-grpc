#![allow(
    dead_code,
    unused_mut,
    unused_variables,
    clippy::identity_op,
    clippy::too_many_lines,
    clippy::derivable_impls,
    clippy::unit_arg,
    clippy::derive_partial_eq_without_eq,
    clippy::manual_range_patterns,
    clippy::default_trait_access,
    clippy::semicolon_if_nothing_returned,
    clippy::doc_markdown,
    clippy::wildcard_imports
)]
//! DO NOT MODIFY. Auto-generated file

///  Points are represented as latitude-longitude pairs in the E7 representation
///  (degrees multiplied by 10**7 and rounded to the nearest integer).
#[derive(Clone, PartialEq, Debug)]
pub struct Point {
    pub latitude: i32,
    pub longitude: i32,
}

///  A latitude-longitude rectangle, represented as two diagonally opposite
///  points "lo" and "hi".
#[derive(Clone, PartialEq, Debug)]
pub struct Rectangle {
    ///  One corner of the rectangle.
    pub lo: Point,
    ///  The other corner of the rectangle.
    pub hi: Point,
}

///  A feature names something at a given point.
///
///  If a feature could not be named, the name is empty.
#[derive(Clone, PartialEq, Debug)]
pub struct Feature {
    ///  The name of the feature.
    pub name: ::ntex_grpc::ByteString,
    ///  The point where the feature is detected.
    pub location: Point,
}

///  A RouteNote is a message sent while at a given point.
#[derive(Clone, PartialEq, Debug)]
pub struct RouteNote {
    ///  The location from which the message is sent.
    pub location: Point,
    ///  The message to be sent.
    pub message: ::ntex_grpc::ByteString,
}

///  A RouteSummary is received in response to a RecordRoute rpc.
///
///  It contains the number of individual points received, the number of
///  detected features, and the total distance covered as the cumulative sum of
///  the distance between each point.
#[derive(Clone, PartialEq, Debug)]
pub struct RouteSummary {
    ///  The number of points received.
    pub point_count: i32,
    ///  The number of known features passed while traversing the route.
    pub feature_count: i32,
    ///  The distance covered in metres.
    pub distance: i32,
    ///  The duration of the traversal in seconds.
    pub elapsed_time: i32,
}

/// `RouteGuide` service definition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteGuide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteGuideMethods {
    GetFeature(RouteGuideGetFeatureMethod),
    ListFeatures(RouteGuideListFeaturesMethod),
    RecordRoute(RouteGuideRecordRouteMethod),
    RouteChat(RouteGuideRouteChatMethod),
}

///  Interface exported by the server.
#[derive(Debug, Clone)]
pub struct RouteGuideClient<T>(T);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RouteGuideGetFeatureMethod;

impl ::ntex_grpc::MethodDef for RouteGuideGetFeatureMethod {
    const NAME: &'static str = "GetFeature";
    const PATH: ::ntex_grpc::ByteString =
        ::ntex_grpc::ByteString::from_static("/routeguide.RouteGuide/GetFeature");
    type Input = Point;
    type Output = Feature;
    const CLIENT_STREAMING: bool = false;
    const SERVER_STREAMING: bool = false;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RouteGuideListFeaturesMethod;

impl ::ntex_grpc::MethodDef for RouteGuideListFeaturesMethod {
    const NAME: &'static str = "ListFeatures";
    const PATH: ::ntex_grpc::ByteString =
        ::ntex_grpc::ByteString::from_static("/routeguide.RouteGuide/ListFeatures");
    type Input = Rectangle;
    type Output = Feature;
    const CLIENT_STREAMING: bool = false;
    const SERVER_STREAMING: bool = true;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RouteGuideRecordRouteMethod;

impl ::ntex_grpc::MethodDef for RouteGuideRecordRouteMethod {
    const NAME: &'static str = "RecordRoute";
    const PATH: ::ntex_grpc::ByteString =
        ::ntex_grpc::ByteString::from_static("/routeguide.RouteGuide/RecordRoute");
    type Input = Point;
    type Output = RouteSummary;
    const CLIENT_STREAMING: bool = true;
    const SERVER_STREAMING: bool = false;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RouteGuideRouteChatMethod;

impl ::ntex_grpc::MethodDef for RouteGuideRouteChatMethod {
    const NAME: &'static str = "RouteChat";
    const PATH: ::ntex_grpc::ByteString =
        ::ntex_grpc::ByteString::from_static("/routeguide.RouteGuide/RouteChat");
    type Input = RouteNote;
    type Output = RouteNote;
    const CLIENT_STREAMING: bool = true;
    const SERVER_STREAMING: bool = true;
}

mod _priv_impl_route_guide {
    use super::*;

    impl ::ntex_grpc::Message for Point {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytePages) {
            ::ntex_grpc::NativeType::serialize(
                &self.latitude,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.longitude,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            let mut msg = Self::default();
            ::ntex_grpc::Message::merge_from(&mut msg, src)?;
            Ok(msg)
        }

        #[inline]
        fn merge_from(
            &mut self,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "Point";
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.latitude,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "latitude"))?,
                    2 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.longitude,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "longitude"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(())
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.latitude,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.longitude,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Point {
        #[inline]
        fn default() -> Self {
            Self {
                latitude: ::core::default::Default::default(),
                longitude: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for Rectangle {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytePages) {
            ::ntex_grpc::NativeType::serialize(
                &self.lo,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.hi,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            let mut msg = Self::default();
            ::ntex_grpc::Message::merge_from(&mut msg, src)?;
            Ok(msg)
        }

        #[inline]
        fn merge_from(
            &mut self,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "Rectangle";
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(&mut self.lo, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "lo"))?,
                    2 => ::ntex_grpc::NativeType::deserialize(&mut self.hi, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "hi"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(())
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.lo,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.hi,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Rectangle {
        #[inline]
        fn default() -> Self {
            Self {
                lo: ::core::default::Default::default(),
                hi: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for Feature {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytePages) {
            ::ntex_grpc::NativeType::serialize(
                &self.name,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.location,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            let mut msg = Self::default();
            ::ntex_grpc::Message::merge_from(&mut msg, src)?;
            Ok(msg)
        }

        #[inline]
        fn merge_from(
            &mut self,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "Feature";
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(&mut self.name, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, "name"))?,
                    2 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.location,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "location"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(())
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.name,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.location,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for Feature {
        #[inline]
        fn default() -> Self {
            Self {
                name: ::core::default::Default::default(),
                location: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for RouteNote {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytePages) {
            ::ntex_grpc::NativeType::serialize(
                &self.location,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.message,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            let mut msg = Self::default();
            ::ntex_grpc::Message::merge_from(&mut msg, src)?;
            Ok(msg)
        }

        #[inline]
        fn merge_from(
            &mut self,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "RouteNote";
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.location,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "location"))?,
                    2 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.message,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "message"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(())
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.location,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.message,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for RouteNote {
        #[inline]
        fn default() -> Self {
            Self {
                location: ::core::default::Default::default(),
                message: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::Message for RouteSummary {
        #[inline]
        fn write(&self, dst: &mut ::ntex_grpc::BytePages) {
            ::ntex_grpc::NativeType::serialize(
                &self.point_count,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.feature_count,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.distance,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
            ::ntex_grpc::NativeType::serialize(
                &self.elapsed_time,
                4,
                ::ntex_grpc::types::DefaultValue::Default,
                dst,
            );
        }

        #[inline]
        fn read(
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
            let mut msg = Self::default();
            ::ntex_grpc::Message::merge_from(&mut msg, src)?;
            Ok(msg)
        }

        #[inline]
        fn merge_from(
            &mut self,
            src: &mut ::ntex_grpc::Bytes,
        ) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {
            const STRUCT_NAME: &str = "RouteSummary";
            while !src.is_empty() {
                let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                match tag {
                    1 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.point_count,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "point_count"))?,
                    2 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.feature_count,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "feature_count"))?,
                    3 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.distance,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "distance"))?,
                    4 => ::ntex_grpc::NativeType::deserialize(
                        &mut self.elapsed_time,
                        tag,
                        wire_type,
                        src,
                    )
                    .map_err(|err| err.push(STRUCT_NAME, "elapsed_time"))?,
                    _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
                }
            }
            Ok(())
        }

        #[inline]
        fn encoded_len(&self) -> usize {
            0 + ::ntex_grpc::NativeType::serialized_len(
                &self.point_count,
                1,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.feature_count,
                2,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.distance,
                3,
                ::ntex_grpc::types::DefaultValue::Default,
            ) + ::ntex_grpc::NativeType::serialized_len(
                &self.elapsed_time,
                4,
                ::ntex_grpc::types::DefaultValue::Default,
            )
        }
    }

    impl ::std::default::Default for RouteSummary {
        #[inline]
        fn default() -> Self {
            Self {
                point_count: ::core::default::Default::default(),
                feature_count: ::core::default::Default::default(),
                distance: ::core::default::Default::default(),
                elapsed_time: ::core::default::Default::default(),
            }
        }
    }

    impl ::ntex_grpc::ServiceDef for RouteGuide {
        const NAME: &'static str = "routeguide.RouteGuide";
        type Methods = RouteGuideMethods;

        #[inline]
        fn method_by_name(name: &str) -> Option<Self::Methods> {
            use ::ntex_grpc::MethodDef;
            match name {
                RouteGuideGetFeatureMethod::NAME => {
                    Some(RouteGuideMethods::GetFeature(RouteGuideGetFeatureMethod))
                }
                RouteGuideListFeaturesMethod::NAME => Some(RouteGuideMethods::ListFeatures(
                    RouteGuideListFeaturesMethod,
                )),
                RouteGuideRecordRouteMethod::NAME => {
                    Some(RouteGuideMethods::RecordRoute(RouteGuideRecordRouteMethod))
                }
                RouteGuideRouteChatMethod::NAME => {
                    Some(RouteGuideMethods::RouteChat(RouteGuideRouteChatMethod))
                }
                _ => None,
            }
        }
    }

    impl<T> RouteGuideClient<T> {
        #[inline]
        /// Create new client instance
        pub fn new(transport: T) -> Self {
            Self(transport)
        }
    }

    impl<T> ::ntex_grpc::client::ClientInformation<T> for RouteGuideClient<T> {
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

    impl<T> RouteGuideClient<T> {
        ///  A simple RPC.
        ///
        ///  Obtains the feature at a given position.
        pub fn get_feature<'a>(
            &'a self,
            req: &'a super::Point,
        ) -> ::ntex_grpc::client::Request<'a, T, RouteGuideGetFeatureMethod> {
            ::ntex_grpc::client::Request::new(&self.0, req)
        }
    }

    impl<T> RouteGuideClient<T> {
        ///  A server-to-client streaming RPC.
        ///
        ///  Obtains the Features available within the given Rectangle.
        pub fn list_features<'a>(
            &'a self,
            req: &'a super::Rectangle,
        ) -> ::ntex_grpc::client::StreamRequest<
            'a,
            T,
            RouteGuideListFeaturesMethod,
            ::ntex_grpc::client::ServerStreaming<'a, super::Rectangle>,
        > {
            ::ntex_grpc::client::StreamRequest::server_streaming(&self.0, req)
        }
    }

    impl<T> RouteGuideClient<T> {
        ///  A client-to-server streaming RPC.
        ///
        ///  Accepts a stream of Points on a route being traversed, returning a
        ///  RouteSummary when traversal is completed.
        pub fn record_route(
            &self,
            tx: &::ntex_grpc::SendStream<super::Point>,
        ) -> ::ntex_grpc::client::StreamRequest<
            '_,
            T,
            RouteGuideRecordRouteMethod,
            ::ntex_grpc::client::ClientStreaming,
        > {
            ::ntex_grpc::client::StreamRequest::client_streaming(&self.0, tx)
        }
    }

    impl<T> RouteGuideClient<T> {
        ///  A Bidirectional streaming RPC.
        ///
        ///  Accepts a stream of RouteNotes sent while a route is being traversed,
        ///  while receiving other RouteNotes (e.g. from other users).
        pub fn route_chat(
            &self,
        ) -> ::ntex_grpc::client::StreamRequest<
            '_,
            T,
            RouteGuideRouteChatMethod,
            ::ntex_grpc::client::Bidi,
        > {
            ::ntex_grpc::client::StreamRequest::bidi(&self.0)
        }
    }
}
