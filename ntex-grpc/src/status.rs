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
            #[inline]
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $($name::$var => stringify!($var)),+
                }
            }
            #[inline]
            pub const fn code(&self) -> u8 {
                match self {
                    $($name::$var => $val),+
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
                HeaderValue::from_static(v.as_str())
            }
        }
    };
}

gen_error_code! {
    #[derive(Copy, Clone, PartialEq, Eq, Debug)]
    pub enum GrpcStatus {
        Ok = 0,
        Cancelled = 1,
        Unknown = 2,
        InvalidArgument = 3,
        DeadlineExceeded = 4,
        NotFound = 5,
        AlredyExists = 6,
        PermissionDenied = 7,
        ResourceExhausted = 8,
        FailedPrecondition = 9,
        Aborted = 10,
        OutOfRange = 11,
        Unimplemented = 12,
        Internal = 13,
        Unavailable = 14,
        DataLoss = 15,
        Unauthenticated = 16
    }
}

impl From<Reason> for GrpcStatus {
    fn from(reason: Reason) -> GrpcStatus {
        match reason {
            Reason::NO_ERROR => GrpcStatus::Internal,
            Reason::PROTOCOL_ERROR => GrpcStatus::Internal,
            Reason::INTERNAL_ERROR => GrpcStatus::Internal,
            Reason::FLOW_CONTROL_ERROR => GrpcStatus::Internal,
            Reason::SETTINGS_TIMEOUT => GrpcStatus::Internal,
            Reason::FRAME_SIZE_ERROR => GrpcStatus::Internal,
            Reason::REFUSED_STREAM => GrpcStatus::Unavailable,
            Reason::CANCEL => GrpcStatus::Cancelled,
            Reason::COMPRESSION_ERROR => GrpcStatus::Internal,
            Reason::CONNECT_ERROR => GrpcStatus::Internal,
            Reason::ENHANCE_YOUR_CALM => GrpcStatus::ResourceExhausted,
            Reason::INADEQUATE_SECURITY => GrpcStatus::PermissionDenied,
            _ => GrpcStatus::Unknown,
        }
    }
}
