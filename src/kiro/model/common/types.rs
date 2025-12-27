//! 辅助结构体定义
//!
//! 定义 Kiro API 使用的辅助结构体，用于响应事件的嵌套字段

use serde::{Deserialize, Serialize};

use super::enums::UserIntent;

/// 内容范围标记
///
/// 用于标记内容在响应中的位置范围
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentSpan {
    /// 起始位置
    pub start: i32,
    /// 结束位置
    pub end: i32,
}

/// 补充网页链接
///
/// 助手响应中包含的相关网页链接
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupplementaryWebLink {
    /// 链接 URL
    pub url: String,
    /// 链接标题
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 链接摘要
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// 相关性评分
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// 最相关的错过的替代方案
///
/// 当存在更好的替代方案时，提供相关信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MostRelevantMissedAlternative {
    /// 替代方案 URL
    pub url: String,
    /// 许可证名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_name: Option<String>,
    /// 仓库名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

/// 代码引用
///
/// 助手响应中引用的代码来源信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    /// 许可证名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_name: Option<String>,
    /// 仓库名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// 引用 URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 附加信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information: Option<String>,
    /// 推荐内容在响应中的位置范围
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation_content_span: Option<ContentSpan>,
    /// 最相关的错过的替代方案
    #[serde(skip_serializing_if = "Option::is_none")]
    pub most_relevant_missed_alternative: Option<MostRelevantMissedAlternative>,
}

impl Reference {
    /// 创建新的空引用
    pub fn new() -> Self {
        Self {
            license_name: None,
            repository: None,
            url: None,
            information: None,
            recommendation_content_span: None,
            most_relevant_missed_alternative: None,
        }
    }
}

impl Default for Reference {
    fn default() -> Self {
        Self::new()
    }
}

/// 后续提示
///
/// 助手建议的后续对话提示
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowupPrompt {
    /// 提示内容
    pub content: String,
    /// 用户意图
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<UserIntent>,
}

/// 编程语言
///
/// 表示代码的编程语言信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgrammingLanguage {
    /// 语言名称
    pub language_name: String,
}

/// 定制化配置
///
/// 自定义模型配置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customization {
    /// ARN (Amazon Resource Name)
    pub arn: String,
    /// 配置名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// 代码查询
///
/// 代码查询信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeQuery {
    /// 代码查询 ID
    pub code_query_id: String,
    /// 编程语言
    #[serde(skip_serializing_if = "Option::is_none")]
    pub programming_language: Option<ProgrammingLanguage>,
    /// 用户输入消息 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_input_message_id: Option<String>,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supplementary_web_link_deserialize() {
        let json = r#"{"url":"https://test.com","title":"Test","snippet":"A test link","score":0.8}"#;
        let link: SupplementaryWebLink = serde_json::from_str(json).unwrap();
        assert_eq!(link.url, "https://test.com");
        assert_eq!(link.title, Some("Test".to_string()));
        assert_eq!(link.snippet, Some("A test link".to_string()));
        assert_eq!(link.score, Some(0.8));
    }
}
