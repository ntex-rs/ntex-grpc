use ntex_h2::frame::Reason;

#[derive(thiserror::Error, Debug)]
pub enum ServiceError {
    #[error("Canceled")]
    Canceled,
    #[error("{0}")]
    ProstEncoder(#[from] prost::EncodeError),
    #[error("Http2 stream has been reset: {0}")]
    H2Reset(Reason),
}
