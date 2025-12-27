//! CRC32 校验实现
//!
//! AWS Event Stream 使用 CRC32C (Castagnoli/ISCSI) 多项式

use crc::{Crc, CRC_32_ISCSI};

/// CRC32C 计算器实例
const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// 计算 CRC32C 校验和
///
/// # Arguments
/// * `data` - 要计算校验和的数据
///
/// # Returns
/// CRC32C 校验和值
pub fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}

/// 验证 CRC32C 校验和
///
/// # Arguments
/// * `data` - 要验证的数据
/// * `expected` - 期望的校验和值
///
/// # Returns
/// 如果校验和匹配返回 true，否则返回 false
pub fn verify_crc32c(data: &[u8], expected: u32) -> bool {
    crc32c(data) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32c_empty() {
        // 空数据的 CRC32C 应该是 0
        assert_eq!(crc32c(&[]), 0);
    }

    #[test]
    fn test_crc32c_known_value() {
        // "123456789" 的 CRC32C 值是 0xE3069283
        let data = b"123456789";
        assert_eq!(crc32c(data), 0xE3069283);
    }

    #[test]
    fn test_verify_crc32c() {
        let data = b"test data";
        let checksum = crc32c(data);
        assert!(verify_crc32c(data, checksum));
        assert!(!verify_crc32c(data, checksum + 1));
    }
}
