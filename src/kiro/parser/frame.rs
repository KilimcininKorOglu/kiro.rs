//! AWS Event Stream message frame parsing
//!
//! ## Message Format
//!
//! ```text
//! ┌──────────────┬──────────────┬──────────────┬──────────┬──────────┬───────────┐
//! │ Total Length │ Header Length│ Prelude CRC  │ Headers  │ Payload  │ Msg CRC   │
//! │   (4 bytes)  │   (4 bytes)  │   (4 bytes)  │ (var)    │ (var)    │ (4 bytes) │
//! └──────────────┴──────────────┴──────────────┴──────────┴──────────┴───────────┘
//! ```
//!
//! - Total Length: Total length of the entire message (including its own 4 bytes)
//! - Header Length: Length of header data
//! - Prelude CRC: CRC32 checksum of first 8 bytes (Total Length + Header Length)
//! - Headers: Header data
//! - Payload: Payload data (usually JSON)
//! - Message CRC: CRC32 checksum of entire message (excluding Message CRC itself)

use super::crc::crc32;
use super::error::{ParseError, ParseResult};
use super::header::{Headers, parse_headers};

/// Prelude fixed size (12 bytes)
pub const PRELUDE_SIZE: usize = 12;

/// Minimum message size (Prelude + Message CRC)
pub const MIN_MESSAGE_SIZE: usize = PRELUDE_SIZE + 4;

/// Maximum message size limit (16 MB)
pub const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

/// Parsed message frame
#[derive(Debug, Clone)]
pub struct Frame {
    /// Message headers
    pub headers: Headers,
    /// Message payload
    pub payload: Vec<u8>,
}

impl Frame {
    /// Get message type
    pub fn message_type(&self) -> Option<&str> {
        self.headers.message_type()
    }

    /// Get event type
    pub fn event_type(&self) -> Option<&str> {
        self.headers.event_type()
    }

    /// Parse payload as JSON
    pub fn payload_as_json<T: serde::de::DeserializeOwned>(&self) -> ParseResult<T> {
        serde_json::from_slice(&self.payload).map_err(ParseError::PayloadDeserialize)
    }

    /// Parse payload as string
    pub fn payload_as_str(&self) -> String {
        String::from_utf8_lossy(&self.payload).to_string()
    }
}

/// Try to parse a complete frame from buffer
///
/// This is a stateless pure function, each call parses independently.
/// Buffer management is handled by the upper layer `EventStreamDecoder`.
///
/// # Arguments
/// * `buffer` - Input buffer
///
/// # Returns
/// - `Ok(Some((frame, consumed)))` - Successfully parsed, returns frame and consumed bytes
/// - `Ok(None)` - Insufficient data, need more data
/// - `Err(e)` - Parse error
pub fn parse_frame(buffer: &[u8]) -> ParseResult<Option<(Frame, usize)>> {
    // Check if there's enough data to read prelude
    if buffer.len() < PRELUDE_SIZE {
        return Ok(None);
    }

    // Read prelude
    let total_length = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    let header_length = u32::from_be_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]);
    let prelude_crc = u32::from_be_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]);

    // Validate message length range
    if total_length < MIN_MESSAGE_SIZE as u32 {
        return Err(ParseError::MessageTooSmall {
            length: total_length,
            min: MIN_MESSAGE_SIZE as u32,
        });
    }

    if total_length > MAX_MESSAGE_SIZE {
        return Err(ParseError::MessageTooLarge {
            length: total_length,
            max: MAX_MESSAGE_SIZE,
        });
    }

    let total_length = total_length as usize;
    let header_length = header_length as usize;

    // Check if we have the complete message
    if buffer.len() < total_length {
        return Ok(None);
    }

    // Verify Prelude CRC
    let actual_prelude_crc = crc32(&buffer[..8]);
    if actual_prelude_crc != prelude_crc {
        return Err(ParseError::PreludeCrcMismatch {
            expected: prelude_crc,
            actual: actual_prelude_crc,
        });
    }

    // Read Message CRC
    let message_crc = u32::from_be_bytes([
        buffer[total_length - 4],
        buffer[total_length - 3],
        buffer[total_length - 2],
        buffer[total_length - 1],
    ]);

    // Verify Message CRC (for entire message excluding last 4 bytes)
    let actual_message_crc = crc32(&buffer[..total_length - 4]);
    if actual_message_crc != message_crc {
        return Err(ParseError::MessageCrcMismatch {
            expected: message_crc,
            actual: actual_message_crc,
        });
    }

    // Parse headers
    let headers_start = PRELUDE_SIZE;
    let headers_end = headers_start + header_length;

    // Validate header boundaries
    if headers_end > total_length - 4 {
        return Err(ParseError::HeaderParseFailed(
            "Header length exceeds message boundary".to_string(),
        ));
    }

    let headers = parse_headers(&buffer[headers_start..headers_end], header_length)?;

    // Extract payload (excluding last 4 bytes of message_crc)
    let payload_start = headers_end;
    let payload_end = total_length - 4;
    let payload = buffer[payload_start..payload_end].to_vec();

    Ok(Some((Frame { headers, payload }, total_length)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_insufficient_data() {
        let buffer = [0u8; 10]; // Less than PRELUDE_SIZE
        assert!(matches!(parse_frame(&buffer), Ok(None)));
    }

    #[test]
    fn test_frame_message_too_small() {
        // Construct a prelude with total_length = 10 (less than minimum)
        let mut buffer = vec![0u8; 16];
        buffer[0..4].copy_from_slice(&10u32.to_be_bytes()); // total_length
        buffer[4..8].copy_from_slice(&0u32.to_be_bytes()); // header_length
        let prelude_crc = crc32(&buffer[0..8]);
        buffer[8..12].copy_from_slice(&prelude_crc.to_be_bytes());

        let result = parse_frame(&buffer);
        assert!(matches!(result, Err(ParseError::MessageTooSmall { .. })));
    }
}
