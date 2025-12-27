//! Token 计算模块
//!
//! 提供文本 token 数量计算功能。
//!
//! # 计算规则
//! - 中文/东亚字符：每个计 3.5 个字符单位
//! - 其他字符：每个计 1 个字符单位
//! - 3 个字符单位 = 1 token（四舍五入）

/// 判断字符是否为东亚字符（中文、日文、韩文等）
///
/// 包含以下 Unicode 范围：
/// - CJK 统一汉字基本区: U+4E00..U+9FFF
/// - CJK 扩展 A: U+3400..U+4DBF
/// - CJK 扩展 B-H: U+20000..U+323AF (多个不连续区间)
/// - CJK 部首补充: U+2E80..U+2EFF
/// - 康熙部首: U+2F00..U+2FDF
/// - 表意文字描述字符: U+2FF0..U+2FFF
/// - CJK 兼容性汉字补充: U+2F800..U+2FA1F
/// - 日文假名: U+3040..U+30FF (平假名、片假名)
/// - 韩文字母: U+AC00..U+D7AF, U+1100..U+11FF
/// - 全角标点: U+FF00..U+FFEF
/// - 中日韩标点: U+3000..U+303F
fn is_east_asian_char(c: char) -> bool {
    matches!(c,
        // CJK 统一汉字基本区
        '\u{4E00}'..='\u{9FFF}' |
        // CJK 扩展 A
        '\u{3400}'..='\u{4DBF}' |
        // CJK 部首补充
        '\u{2E80}'..='\u{2EFF}' |
        // 康熙部首
        '\u{2F00}'..='\u{2FDF}' |
        // 表意文字描述字符
        '\u{2FF0}'..='\u{2FFF}' |
        // CJK 扩展 B
        '\u{20000}'..='\u{2A6DF}' |
        // CJK 扩展 C
        '\u{2A700}'..='\u{2B73F}' |
        // CJK 扩展 D
        '\u{2B740}'..='\u{2B81F}' |
        // CJK 扩展 E
        '\u{2B820}'..='\u{2CEAF}' |
        // CJK 扩展 F
        '\u{2CEB0}'..='\u{2EBEF}' |
        // CJK 兼容性汉字补充
        '\u{2F800}'..='\u{2FA1F}' |
        // CJK 扩展 G
        '\u{30000}'..='\u{3134F}' |
        // CJK 扩展 H
        '\u{31350}'..='\u{323AF}' |
        // 日文平假名
        '\u{3040}'..='\u{309F}' |
        // 日文片假名
        '\u{30A0}'..='\u{30FF}' |
        // 韩文音节
        '\u{AC00}'..='\u{D7AF}' |
        // 韩文字母
        '\u{1100}'..='\u{11FF}' |
        // 全角 ASCII 和标点
        '\u{FF00}'..='\u{FFEF}' |
        // 中日韩标点符号
        '\u{3000}'..='\u{303F}'
    )
}

/// 计算文本的 token 数量
///
/// # 计算规则
/// - 中文/东亚字符：每个计 3.5 个字符单位
/// - 其他字符：每个计 1 个字符单位
/// - 3 个字符单位 = 1 token（四舍五入）
///
/// # 实现细节
/// 为避免浮点精度问题，内部使用 2 倍放大：
/// - 中文字符 = 7 单位，普通字符 = 2 单位，6 单位 = 1 token
///
/// # 示例
/// ```
/// use kiro_rs::anthropic::token::count_tokens;
///
/// // "你" = 3.5 字符单位 ≈ 1 token
/// assert_eq!(count_tokens("你"), 1);
///
/// // "abc" = 3 字符单位 = 1 token
/// assert_eq!(count_tokens("abc"), 1);
///
/// // "你好" = 7 字符单位 ≈ 2 tokens
/// assert_eq!(count_tokens("你好"), 2);
/// ```
pub fn count_tokens(text: &str) -> u64 {
    // 使用 2 倍放大避免浮点精度问题
    // 中文 = 7 (3.5 × 2), 普通 = 2 (1 × 2), 除数 = 6 (3 × 2)
    let char_units: u64 = text
        .chars()
        .map(|c| if is_east_asian_char(c) { 7 } else { 2 })
        .sum();

    // 四舍五入: (n + 3) / 6
    (char_units + 3) / 6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_string() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_chinese_only() {
        // 1 个中文 = 3.5 字符单位 ≈ 1 token (3.5/3=1.17)
        assert_eq!(count_tokens("你"), 1);
        // 2 个中文 = 7 字符单位 ≈ 2 tokens (7/3=2.33)
        assert_eq!(count_tokens("你好"), 2);
        // 3 个中文 = 10.5 字符单位 ≈ 4 tokens (10.5/3=3.5)
        assert_eq!(count_tokens("你好世"), 4);
    }

    #[test]
    fn test_ascii_only() {
        // 3 个 ASCII = 3 字符单位 = 1 token
        assert_eq!(count_tokens("abc"), 1);
        // 6 个 ASCII = 6 字符单位 = 2 tokens
        assert_eq!(count_tokens("abcdef"), 2);
        // 9 个 ASCII = 9 字符单位 = 3 tokens
        assert_eq!(count_tokens("abcdefghi"), 3);
    }

    #[test]
    fn test_mixed() {
        // "你好abc" = 7+3 = 10 字符单位 ≈ 3 tokens (10/3=3.33)
        assert_eq!(count_tokens("你好abc"), 3);
        // "a你" = 1+3.5 = 4.5 字符单位 ≈ 2 tokens (4.5/3=1.5)
        assert_eq!(count_tokens("a你"), 2);
    }

    #[test]
    fn test_rounding() {
        // 1 字符单位: 1/3=0.33 → 0
        assert_eq!(count_tokens("a"), 0);
        // 2 字符单位: 2/3=0.67 → 1
        assert_eq!(count_tokens("ab"), 1);
        // 3 字符单位: 3/3=1.0 → 1
        assert_eq!(count_tokens("abc"), 1);
        // 4 字符单位: 4/3=1.33 → 1
        assert_eq!(count_tokens("abcd"), 1);
        // 5 字符单位: 5/3=1.67 → 2
        assert_eq!(count_tokens("abcde"), 2);
        // 6 字符单位: 6/3=2.0 → 2
        assert_eq!(count_tokens("abcdef"), 2);
    }

    #[test]
    fn test_japanese() {
        // 平假名 "あ" = 3.5 字符单位 ≈ 1 token
        assert_eq!(count_tokens("あ"), 1);
        // 片假名 "アイ" = 7 字符单位 ≈ 2 tokens
        assert_eq!(count_tokens("アイ"), 2);
    }

    #[test]
    fn test_korean() {
        // 韩文 "안" = 3.5 字符单位 ≈ 1 token
        assert_eq!(count_tokens("안"), 1);
        // 韩文 "안녕" = 7 字符单位 ≈ 2 tokens
        assert_eq!(count_tokens("안녕"), 2);
    }

    #[test]
    fn test_fullwidth() {
        // 全角字符 "Ａ" = 3.5 字符单位 ≈ 1 token
        assert_eq!(count_tokens("Ａ"), 1);
        // 全角字符 "ＡＢ" = 7 字符单位 ≈ 2 tokens
        assert_eq!(count_tokens("ＡＢ"), 2);
    }

    #[test]
    fn test_cjk_punctuation() {
        // 中日韩标点 "。" = 3.5 字符单位 ≈ 1 token
        assert_eq!(count_tokens("。"), 1);
        // 中日韩标点 "。、" = 7 字符单位 ≈ 2 tokens
        assert_eq!(count_tokens("。、"), 2);
    }
}
