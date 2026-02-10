//! AWS Event Stream header parsing
//!
//! Implements header parsing functionality for the AWS Event Stream protocol

use super::error::{ParseError, ParseResult};
use std::collections::HashMap;

/// Header value type identifier
///
/// 10 value types defined by the AWS Event Stream protocol
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderValueType {
    BoolTrue = 0,
    BoolFalse = 1,
    Byte = 2,
    Short = 3,
    Integer = 4,
    Long = 5,
    ByteArray = 6,
    String = 7,
    Timestamp = 8,
    Uuid = 9,
}

impl TryFrom<u8> for HeaderValueType {
    type Error = ParseError;

    fn try_from(value: u8) -> ParseResult<Self> {
        match value {
            0 => Ok(Self::BoolTrue),
            1 => Ok(Self::BoolFalse),
            2 => Ok(Self::Byte),
            3 => Ok(Self::Short),
            4 => Ok(Self::Integer),
            5 => Ok(Self::Long),
            6 => Ok(Self::ByteArray),
            7 => Ok(Self::String),
            8 => Ok(Self::Timestamp),
            9 => Ok(Self::Uuid),
            _ => Err(ParseError::InvalidHeaderType(value)),
        }
    }
}

/// Header value
///
/// Supports all value types defined by the AWS Event Stream protocol
#[derive(Debug, Clone, PartialEq)]
pub enum HeaderValue {
    Bool(bool),
    Byte(i8),
    Short(i16),
    Integer(i32),
    Long(i64),
    ByteArray(Vec<u8>),
    String(String),
    Timestamp(i64),
    Uuid([u8; 16]),
}

impl HeaderValue {
    /// Try to get string value
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Message header collection
#[derive(Debug, Clone, Default)]
pub struct Headers {
    inner: HashMap<String, HeaderValue>,
}

impl Headers {
    /// Create empty header collection
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Insert header
    pub fn insert(&mut self, name: String, value: HeaderValue) {
        self.inner.insert(name, value);
    }

    /// Get header value
    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        self.inner.get(name)
    }

    /// Get string type header value
    pub fn get_string(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|v| v.as_str())
    }

    /// Get message type (:message-type)
    pub fn message_type(&self) -> Option<&str> {
        self.get_string(":message-type")
    }

    /// Get event type (:event-type)
    pub fn event_type(&self) -> Option<&str> {
        self.get_string(":event-type")
    }

    /// Get exception type (:exception-type)
    pub fn exception_type(&self) -> Option<&str> {
        self.get_string(":exception-type")
    }

    /// Get error code (:error-code)
    pub fn error_code(&self) -> Option<&str> {
        self.get_string(":error-code")
    }
}

/// Parse headers from byte stream
///
/// # Arguments
/// * `data` - Header data slice
/// * `header_length` - Total header length
///
/// # Returns
/// Parsed Headers structure
pub fn parse_headers(data: &[u8], header_length: usize) -> ParseResult<Headers> {
    // Verify data length is sufficient
    if data.len() < header_length {
        return Err(ParseError::Incomplete {
            needed: header_length,
            available: data.len(),
        });
    }

    let mut headers = Headers::new();
    let mut offset = 0;

    while offset < header_length {
        // Read header name length (1 byte)
        if offset >= data.len() {
            break;
        }
        let name_len = data[offset] as usize;
        offset += 1;

        // Validate name length
        if name_len == 0 {
            return Err(ParseError::HeaderParseFailed(
                "Header name length cannot be 0".to_string(),
            ));
        }

        // Read header name
        if offset + name_len > data.len() {
            return Err(ParseError::Incomplete {
                needed: name_len,
                available: data.len() - offset,
            });
        }
        let name = String::from_utf8_lossy(&data[offset..offset + name_len]).to_string();
        offset += name_len;

        // Read value type (1 byte)
        if offset >= data.len() {
            return Err(ParseError::Incomplete {
                needed: 1,
                available: 0,
            });
        }
        let value_type = HeaderValueType::try_from(data[offset])?;
        offset += 1;

        // Parse value based on type
        let value = parse_header_value(&data[offset..], value_type, &mut offset)?;
        headers.insert(name, value);
    }

    Ok(headers)
}

/// Parse header value
fn parse_header_value(
    data: &[u8],
    value_type: HeaderValueType,
    global_offset: &mut usize,
) -> ParseResult<HeaderValue> {
    let mut local_offset = 0;

    let result = match value_type {
        HeaderValueType::BoolTrue => Ok(HeaderValue::Bool(true)),
        HeaderValueType::BoolFalse => Ok(HeaderValue::Bool(false)),
        HeaderValueType::Byte => {
            ensure_bytes(data, 1)?;
            let v = data[0] as i8;
            local_offset = 1;
            Ok(HeaderValue::Byte(v))
        }
        HeaderValueType::Short => {
            ensure_bytes(data, 2)?;
            let v = i16::from_be_bytes([data[0], data[1]]);
            local_offset = 2;
            Ok(HeaderValue::Short(v))
        }
        HeaderValueType::Integer => {
            ensure_bytes(data, 4)?;
            let v = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            local_offset = 4;
            Ok(HeaderValue::Integer(v))
        }
        HeaderValueType::Long => {
            ensure_bytes(data, 8)?;
            let v = i64::from_be_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            local_offset = 8;
            Ok(HeaderValue::Long(v))
        }
        HeaderValueType::Timestamp => {
            ensure_bytes(data, 8)?;
            let v = i64::from_be_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            local_offset = 8;
            Ok(HeaderValue::Timestamp(v))
        }
        HeaderValueType::ByteArray => {
            ensure_bytes(data, 2)?;
            let len = u16::from_be_bytes([data[0], data[1]]) as usize;
            ensure_bytes(data, 2 + len)?;
            let v = data[2..2 + len].to_vec();
            local_offset = 2 + len;
            Ok(HeaderValue::ByteArray(v))
        }
        HeaderValueType::String => {
            ensure_bytes(data, 2)?;
            let len = u16::from_be_bytes([data[0], data[1]]) as usize;
            ensure_bytes(data, 2 + len)?;
            let v = String::from_utf8_lossy(&data[2..2 + len]).to_string();
            local_offset = 2 + len;
            Ok(HeaderValue::String(v))
        }
        HeaderValueType::Uuid => {
            ensure_bytes(data, 16)?;
            let mut uuid = [0u8; 16];
            uuid.copy_from_slice(&data[..16]);
            local_offset = 16;
            Ok(HeaderValue::Uuid(uuid))
        }
    };

    *global_offset += local_offset;
    result
}

/// Ensure sufficient bytes available
fn ensure_bytes(data: &[u8], needed: usize) -> ParseResult<()> {
    if data.len() < needed {
        Err(ParseError::Incomplete {
            needed,
            available: data.len(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_value_type_conversion() {
        assert_eq!(
            HeaderValueType::try_from(0).unwrap(),
            HeaderValueType::BoolTrue
        );
        assert_eq!(
            HeaderValueType::try_from(7).unwrap(),
            HeaderValueType::String
        );
        assert!(HeaderValueType::try_from(10).is_err());
    }

    #[test]
    fn test_header_value_as_str() {
        let value = HeaderValue::String("test".to_string());
        assert_eq!(value.as_str(), Some("test"));

        let value = HeaderValue::Bool(true);
        assert_eq!(value.as_str(), None);
    }

    #[test]
    fn test_headers_get_string() {
        let mut headers = Headers::new();
        headers.insert(
            ":message-type".to_string(),
            HeaderValue::String("event".to_string()),
        );
        assert_eq!(headers.message_type(), Some("event"));
    }

    #[test]
    fn test_parse_headers_string() {
        // Construct a simple header: name_len(1) + name + type(7=string) + value_len(2) + value
        // Header name: "x" (length 1)
        // Value type: 7 (String)
        // Value: "ab" (length 2)
        let data = [1u8, b'x', 7, 0, 2, b'a', b'b'];
        let headers = parse_headers(&data, data.len()).unwrap();
        assert_eq!(headers.get_string("x"), Some("ab"));
    }
}
