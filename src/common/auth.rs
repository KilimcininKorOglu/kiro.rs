//! 公共认证工具函数

use axum::{
    body::Body,
    http::{header, Request},
};

/// 从请求中提取 API Key
///
/// 支持两种认证方式：
/// - `x-api-key` header
/// - `Authorization: Bearer <token>` header
pub fn extract_api_key(request: &Request<Body>) -> Option<String> {
    // 优先检查 x-api-key
    if let Some(key) = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
    {
        return Some(key.to_string());
    }

    // 其次检查 Authorization: Bearer
    request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// 常量时间字符串比较，防止时序攻击
///
/// 无论字符串内容如何，比较所需的时间都是恒定的，
/// 这可以防止攻击者通过测量响应时间来猜测 API Key。
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    // 长度不同时仍然遍历完整的比较，以保持恒定时间
    if a_bytes.len() != b_bytes.len() {
        // 遍历较长的字符串以保持恒定时间
        let max_len = a_bytes.len().max(b_bytes.len());
        let mut _dummy = 0u8;
        for i in 0..max_len {
            let x = a_bytes.get(i).copied().unwrap_or(0);
            let y = b_bytes.get(i).copied().unwrap_or(0);
            _dummy |= x ^ y;
        }
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a_bytes.iter().zip(b_bytes.iter()) {
        result |= x ^ y;
    }
    result == 0
}
