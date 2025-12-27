//! 计费事件
//!
//! 处理 meteringEvent 类型的事件

use serde::Deserialize;

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::{EventPayload, EventType};

/// 计费事件
///
/// 包含本次请求的资源使用计费信息
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeteringEvent {
    /// 计费单位 (单数形式)
    #[serde(default)]
    pub unit: String,
    /// 计费单位 (复数形式)
    #[serde(default)]
    pub unit_plural: String,
    /// 使用量
    #[serde(default)]
    pub usage: f64,
}

impl EventPayload for MeteringEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }

    fn event_type() -> EventType {
        EventType::Metering
    }
}

impl MeteringEvent {
    /// 获取计费单位
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// 获取计费单位复数形式
    pub fn unit_plural(&self) -> &str {
        &self.unit_plural
    }

    /// 获取使用量
    pub fn usage(&self) -> f64 {
        self.usage
    }

    /// 获取格式化的使用量字符串 (6位小数精度)
    pub fn formatted_usage(&self) -> String {
        self.format_usage_with_precision(6)
    }

    /// 获取格式化的简短使用量字符串 (3位小数精度)
    pub fn formatted_usage_short(&self) -> String {
        self.format_usage_with_precision(3)
    }

    /// 使用指定精度格式化使用量
    fn format_usage_with_precision(&self, precision: usize) -> String {
        let unit = if self.usage == 1.0 {
            &self.unit
        } else {
            &self.unit_plural
        };
        format!("{:.prec$} {}", self.usage, unit, prec = precision)
    }
}

impl std::fmt::Display for MeteringEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.formatted_usage_short())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize() {
        let json = r#"{
            "unit": "credit",
            "unitPlural": "credits",
            "usage": 0.24743150922056384
        }"#;
        let event: MeteringEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.unit(), "credit");
        assert_eq!(event.unit_plural(), "credits");
        assert!((event.usage() - 0.247).abs() < 0.01);
    }

    #[test]
    fn test_formatted_usage() {
        let event = MeteringEvent {
            unit: "credit".to_string(),
            unit_plural: "credits".to_string(),
            usage: 0.247,
        };
        assert!(event.formatted_usage().contains("credits"));
    }

    #[test]
    fn test_singular_unit() {
        let event = MeteringEvent {
            unit: "credit".to_string(),
            unit_plural: "credits".to_string(),
            usage: 1.0,
        };
        assert!(event.formatted_usage().contains("credit"));
        assert!(!event.formatted_usage().contains("credits"));
    }
}
