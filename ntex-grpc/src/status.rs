use ntex_h2::frame::Reason;
use ntex_http::HeaderValue;

macro_rules! gen_error_code {
    (
        $( #[$enum_attr:meta] )*
        pub enum $name:ident {
            $(
                $( #[$enum_item_attr:meta] )*
                    $var:ident=$val:expr
            ),+
        }) => {
        $( #[$enum_attr] )*
        #[repr(u8)]
        pub enum $name {
            $(
                $( #[$enum_item_attr] )*
                    $var = $val
            ),+
        }

        impl $name {
            /// Status name, e.g. `"NotFound"`.
            #[inline]
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $($name::$var => stringify!($var)),+
                }
            }

            /// Status name with a `grpc-status-` prefix, e.g.
            /// `"grpc-status-NotFound"`.
            ///
            /// Used as the error signature of
            /// [`ClientError::GrpcStatus`](crate::client::ClientError::GrpcStatus).
            #[inline]
            pub const fn signature(&self) -> &'static str {
                match self {
                    $($name::$var => concat!("grpc-status-", stringify!($var))),+
                }
            }

            /// Numeric code, e.g. `5` for `NotFound`.
            #[inline]
            pub const fn code(&self) -> u8 {
                match self {
                    $($name::$var => $val),+
                }
            }

            /// Numeric code as text, the way it is sent in `grpc-status`.
            #[inline]
            pub const fn code_str(&self) -> &'static str {
                match self {
                    $($name::$var => stringify!($val)),+
                }
            }
        }

        impl std::convert::TryFrom<u8> for $name {
            type Error = ();
            #[inline]
            fn try_from(v: u8) -> Result<Self, Self::Error> {
                match v {
                    $($val => Ok($name::$var)),+
                    ,_ => Err(())
                }
            }
        }

        impl From<$name> for u8 {
            #[inline]
            fn from(v: $name) -> Self {
                unsafe { ::std::mem::transmute(v) }
            }
        }

        impl From<$name> for HeaderValue {
            #[inline]
            fn from(v: $name) -> Self {
                HeaderValue::from_static(v.code_str())
            }
        }
    };
}

gen_error_code! {
    /// gRPC status code.
    ///
    /// The server sends it in the `grpc-status` trailer. On the client a
    /// status other than `Ok` shows up as
    /// [`ClientError::GrpcStatus`](crate::client::ClientError::GrpcStatus),
    /// or [`ClientError::DeadlineExceeded`](crate::client::ClientError::DeadlineExceeded).
    /// A server returns one with [`ServerError`](crate::server::ServerError).
    #[derive(Copy, Clone, PartialEq, Eq, Debug)]
    pub enum GrpcStatus {
        /// The call succeeded.
        Ok = 0,
        /// The call was cancelled, usually by the caller.
        Cancelled = 1,
        /// An error that fits no other status, e.g. a status from another
        /// system that this one doesn't know.
        Unknown = 2,
        /// The request itself is invalid, whatever the state of the system.
        InvalidArgument = 3,
        /// The deadline passed before the call finished.
        DeadlineExceeded = 4,
        /// The requested entity does not exist.
        NotFound = 5,
        /// The entity the client tried to create already exists.
        AlreadyExists = 6,
        /// The caller is known but not allowed to do this. Use
        /// `Unauthenticated` when the caller could not be identified.
        PermissionDenied = 7,
        /// Something ran out, like a quota or disk space.
        ResourceExhausted = 8,
        /// The system is not in the state the operation needs, e.g. deleting
        /// a directory that is not empty.
        FailedPrecondition = 9,
        /// The operation was aborted, usually by a concurrency conflict.
        Aborted = 10,
        /// The operation went past the valid range, e.g. reading past the
        /// end of a file.
        OutOfRange = 11,
        /// The method is not implemented or not supported.
        Unimplemented = 12,
        /// Something the server relies on is broken.
        Internal = 13,
        /// The service can't be reached right now. Usually temporary, the
        /// call can be retried.
        Unavailable = 14,
        /// Data was lost or corrupted and can't be recovered.
        DataLoss = 15,
        /// The request has no valid credentials.
        Unauthenticated = 16
    }
}

/// Maps an HTTP/2 `RST_STREAM` code to a status, as the gRPC spec says.
///
/// Codes the spec does not list, including unknown ones, map to
/// [`GrpcStatus::Internal`].
impl From<Reason> for GrpcStatus {
    fn from(reason: Reason) -> GrpcStatus {
        match reason {
            Reason::REFUSED_STREAM => GrpcStatus::Unavailable,
            Reason::CANCEL => GrpcStatus::Cancelled,
            Reason::ENHANCE_YOUR_CALM => GrpcStatus::ResourceExhausted,
            Reason::INADEQUATE_SECURITY => GrpcStatus::PermissionDenied,
            _ => GrpcStatus::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_exists() {
        let st = GrpcStatus::try_from(6).unwrap();
        assert_eq!(st, GrpcStatus::AlreadyExists);
        assert_eq!(st.signature(), "grpc-status-AlreadyExists");
    }

    #[test]
    fn status_codes() {
        let all = [
            (GrpcStatus::Ok, 0_u8, "Ok"),
            (GrpcStatus::Cancelled, 1, "Cancelled"),
            (GrpcStatus::Unknown, 2, "Unknown"),
            (GrpcStatus::InvalidArgument, 3, "InvalidArgument"),
            (GrpcStatus::DeadlineExceeded, 4, "DeadlineExceeded"),
            (GrpcStatus::NotFound, 5, "NotFound"),
            (GrpcStatus::AlreadyExists, 6, "AlreadyExists"),
            (GrpcStatus::PermissionDenied, 7, "PermissionDenied"),
            (GrpcStatus::ResourceExhausted, 8, "ResourceExhausted"),
            (GrpcStatus::FailedPrecondition, 9, "FailedPrecondition"),
            (GrpcStatus::Aborted, 10, "Aborted"),
            (GrpcStatus::OutOfRange, 11, "OutOfRange"),
            (GrpcStatus::Unimplemented, 12, "Unimplemented"),
            (GrpcStatus::Internal, 13, "Internal"),
            (GrpcStatus::Unavailable, 14, "Unavailable"),
            (GrpcStatus::DataLoss, 15, "DataLoss"),
            (GrpcStatus::Unauthenticated, 16, "Unauthenticated"),
        ];

        for (status, code, name) in all {
            assert_eq!(status.code(), code);
            assert_eq!(status.as_str(), name);
            assert_eq!(status.signature(), format!("grpc-status-{name}"));
            assert_eq!(status.code_str(), code.to_string());
            assert_eq!(GrpcStatus::try_from(code).unwrap(), status);
            assert_eq!(u8::from(status), code);
            assert_eq!(
                HeaderValue::from(status).as_ref(),
                code.to_string().as_bytes()
            );
        }

        assert!(GrpcStatus::try_from(17).is_err());
        assert!(GrpcStatus::try_from(u8::MAX).is_err());
    }

    #[test]
    fn status_from_reason() {
        let cases = [
            (Reason::NO_ERROR, GrpcStatus::Internal),
            (Reason::PROTOCOL_ERROR, GrpcStatus::Internal),
            (Reason::INTERNAL_ERROR, GrpcStatus::Internal),
            (Reason::FLOW_CONTROL_ERROR, GrpcStatus::Internal),
            (Reason::SETTINGS_TIMEOUT, GrpcStatus::Internal),
            (Reason::FRAME_SIZE_ERROR, GrpcStatus::Internal),
            (Reason::COMPRESSION_ERROR, GrpcStatus::Internal),
            (Reason::CONNECT_ERROR, GrpcStatus::Internal),
            (Reason::REFUSED_STREAM, GrpcStatus::Unavailable),
            (Reason::CANCEL, GrpcStatus::Cancelled),
            (Reason::ENHANCE_YOUR_CALM, GrpcStatus::ResourceExhausted),
            (Reason::INADEQUATE_SECURITY, GrpcStatus::PermissionDenied),
            (Reason::STREAM_CLOSED, GrpcStatus::Internal),
            (Reason::HTTP_1_1_REQUIRED, GrpcStatus::Internal),
            (Reason::from(0xff_u32), GrpcStatus::Internal),
        ];

        for (reason, status) in cases {
            assert_eq!(GrpcStatus::from(reason), status, "{reason:?}");
        }
    }
}
