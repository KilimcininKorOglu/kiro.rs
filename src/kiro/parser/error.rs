//! AWS Event Stream parsing error definitions

use std::fmt;

/// Parse error types
#[derive(Debug)]
pub enum ParseError {
    /// Insufficient data, need more bytes
    Incomplete { needed: usize, available: usize },
    /// Prelude CRC verification failed
    PreludeCrcMismatch { expected: u32, actual: u32 },
    /// Message CRC verification failed
    MessageCrcMismatch { expected: u32, actual: u32 },
    /// Invalid header value type
    InvalidHeaderType(u8),
    /// Header parsing error
    HeaderParseFailed(String),
    /// Message length exceeds limit
    MessageTooLarge { length: u32, max: u32 },
    /// Message length too small
    MessageTooSmall { length: u32, min: u32 },
    /// Invalid message type
    InvalidMessageType(String),
    /// Payload deserialization failed
    PayloadDeserialize(serde_json::Error),
    /// IO error
    Io(std::io::Error),
    /// Too many consecutive errors, decoder stopped
    TooManyErrors { count: usize, last_error: String },
    /// Buffer overflow
    BufferOverflow { size: usize, max: usize },
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete { needed, available } => {
                write!(f, "Insufficient data: need {} bytes, have {} bytes", needed, available)
            }
            Self::PreludeCrcMismatch { expected, actual } => {
                write!(
                    f,
                    "Prelude CRC verification failed: expected 0x{:08x}, actual 0x{:08x}",
                    expected, actual
                )
            }
            Self::MessageCrcMismatch { expected, actual } => {
                write!(
                    f,
                    "Message CRC verification failed: expected 0x{:08x}, actual 0x{:08x}",
                    expected, actual
                )
            }
            Self::InvalidHeaderType(t) => write!(f, "Invalid header value type: {}", t),
            Self::HeaderParseFailed(msg) => write!(f, "Header parsing failed: {}", msg),
            Self::MessageTooLarge { length, max } => {
                write!(f, "Message length exceeds limit: {} bytes (max {})", length, max)
            }
            Self::MessageTooSmall { length, min } => {
                write!(f, "Message length too small: {} bytes (min {})", length, min)
            }
            Self::InvalidMessageType(t) => write!(f, "Invalid message type: {}", t),
            Self::PayloadDeserialize(e) => write!(f, "Payload deserialization failed: {}", e),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::TooManyErrors { count, last_error } => {
                write!(
                    f,
                    "Too many consecutive errors ({} times), decoder stopped: {}",
                    count, last_error
                )
            }
            Self::BufferOverflow { size, max } => {
                write!(f, "Buffer overflow: {} bytes (max {})", size, max)
            }
        }
    }
}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for ParseError {
    fn from(e: serde_json::Error) -> Self {
        Self::PayloadDeserialize(e)
    }
}

/// Parse result type
pub type ParseResult<T> = Result<T, ParseError>;
