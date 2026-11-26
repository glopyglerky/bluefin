/// Errors deliberately exclude imported text, secrets, certificates and URLs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the connection code is malformed")]
    InvalidCode,
    #[error("the connection code uses an unsupported version")]
    UnsupportedVersion,
    #[error("the connection code checksum does not match")]
    Checksum,
    #[error("the connection code has expired")]
    Expired,
    #[error("the connection code has invalid or unsupported fields")]
    InvalidFields,
    #[error("the imported content exceeds the permitted size")]
    TooLarge,
    #[error("the image format is unsupported or damaged")]
    InvalidImage,
    #[error("the image contains no readable connection code")]
    NoCode,
    #[error("the image contains more than one different connection code")]
    AmbiguousImage,
    #[error("the QR theme does not meet contrast or quiet-zone requirements")]
    InvalidTheme,
    #[error("the connection protocol message is invalid")]
    InvalidMessage,
    #[error("the connection service does not match")]
    ServiceMismatch,
    #[error("a requested relay is not permitted by the application policy")]
    RelayNotAllowed,
    #[error("the operating system could not supply cryptographic randomness")]
    Randomness,
    #[error("the post-quantum identity is invalid")]
    InvalidIdentity,
    #[error("the post-quantum cryptographic operation failed")]
    Crypto,
    #[error("the peer did not establish the required pinned post-quantum session")]
    Authentication,
    #[error("the connection timed out")]
    Timeout,
    #[error("the connection is closed")]
    Closed,
    #[error("the network connection failed")]
    Network,
}
pub type Result<T> = std::result::Result<T, Error>;
