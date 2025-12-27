//! CRC32 校验实现
//!
//! AWS Event Stream 使用 CRC32 (ISO-HDLC/以太网/ZIP 标准)

use crc::{Crc, CRC_32_ISO_HDLC};

/// CRC32 计算器实例 (ISO-HDLC 标准，多项式 0xEDB88320)
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

/// 计算 CRC32 校验和 (ISO-HDLC 标准)
///
/// # Arguments
/// * `data` - 要计算校验和的数据
///
/// # Returns
/// CRC32 校验和值
pub fn crc32(data: &[u8]) -> u32 {
    CRC32.checksum(data)
}

/// 验证 CRC32 校验和
///
/// # Arguments
/// * `data` - 要验证的数据
/// * `expected` - 期望的校验和值
///
/// # Returns
/// 如果校验和匹配返回 true，否则返回 false
pub fn verify_crc32(data: &[u8], expected: u32) -> bool {
    crc32(data) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_empty() {
        // 空数据的 CRC32 应该是 0
        assert_eq!(crc32(&[]), 0);
    }

    #[test]
    fn test_crc32_known_value() {
        // "123456789" 的 CRC32 (ISO-HDLC) 值是 0xCBF43926
        let data = b"123456789";
        assert_eq!(crc32(data), 0xCBF43926);
    }

    #[test]
    fn test_verify_crc32() {
        let data = b"test data";
        let checksum = crc32(data);
        assert!(verify_crc32(data, checksum));
        assert!(!verify_crc32(data, checksum + 1));
    }
}
