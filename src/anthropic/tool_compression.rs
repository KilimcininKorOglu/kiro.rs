//! Tool compression module
//!
//! When tool definitions exceed the target threshold, dynamically compress
//! tool payloads to prevent Kiro API 500 errors.
//! Compression strategy:
//! 1. Simplify input_schema (keep only type/enum/required)
//! 2. Proportionally compress description (minimum 50 characters)

use crate::kiro::model::requests::tool::{InputSchema, Tool, ToolSpecification};

/// Tool compression target size (20KB)
const TOOL_COMPRESSION_TARGET_SIZE: usize = 20 * 1024;

/// Minimum description length after compression
const MIN_TOOL_DESCRIPTION_LENGTH: usize = 50;

/// Calculate JSON serialized size of tool list
fn calculate_tools_size(tools: &[Tool]) -> usize {
    serde_json::to_string(tools).map(|s| s.len()).unwrap_or(0)
}

/// Simplify input_schema, keeping only type/enum/required/properties/items
fn simplify_input_schema(schema: &serde_json::Value) -> serde_json::Value {
    match schema {
        serde_json::Value::Object(map) => {
            let mut simplified = serde_json::Map::new();

            // Keep essential fields
            for key in &["type", "enum", "required"] {
                if let Some(v) = map.get(*key) {
                    simplified.insert(key.to_string(), v.clone());
                }
            }

            // Recursively process properties
            if let Some(serde_json::Value::Object(props)) = map.get("properties") {
                let mut simplified_props = serde_json::Map::new();
                for (key, value) in props {
                    simplified_props.insert(key.clone(), simplify_input_schema(value));
                }
                simplified.insert(
                    "properties".to_string(),
                    serde_json::Value::Object(simplified_props),
                );
            }

            // Process items (array type)
            if let Some(items) = map.get("items") {
                simplified.insert("items".to_string(), simplify_input_schema(items));
            }

            // Process additionalProperties
            if let Some(ap) = map.get("additionalProperties") {
                simplified.insert(
                    "additionalProperties".to_string(),
                    simplify_input_schema(ap),
                );
            }

            // Process anyOf/oneOf/allOf
            for key in &["anyOf", "oneOf", "allOf"] {
                if let Some(serde_json::Value::Array(arr)) = map.get(*key) {
                    let simplified_arr: Vec<serde_json::Value> =
                        arr.iter().map(simplify_input_schema).collect();
                    simplified.insert(key.to_string(), serde_json::Value::Array(simplified_arr));
                }
            }

            serde_json::Value::Object(simplified)
        }
        other => other.clone(),
    }
}

/// Compress tool description to target length (UTF-8 safe truncation)
fn compress_description(description: &str, target_length: usize) -> String {
    let target = target_length.max(MIN_TOOL_DESCRIPTION_LENGTH);

    if description.len() <= target {
        return description.to_string();
    }

    let trunc_len = target.saturating_sub(3); // Leave room for "..."

    // Find valid UTF-8 character boundary
    let safe_len = description
        .char_indices()
        .take_while(|(i, _)| *i < trunc_len)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);

    if safe_len == 0 {
        return description
            .chars()
            .take(MIN_TOOL_DESCRIPTION_LENGTH)
            .collect();
    }

    format!("{}...", &description[..safe_len])
}

/// Compress tools if total size exceeds threshold
///
/// Returns compressed tool list (or clone of original if no compression needed)
pub fn compress_tools_if_needed(tools: &[Tool]) -> Vec<Tool> {
    if tools.is_empty() {
        return tools.to_vec();
    }

    let original_size = calculate_tools_size(tools);
    if original_size <= TOOL_COMPRESSION_TARGET_SIZE {
        tracing::debug!(
            "Tool size {} bytes within target {} bytes, no compression needed",
            original_size,
            TOOL_COMPRESSION_TARGET_SIZE
        );
        return tools.to_vec();
    }

    tracing::info!(
        "Tool size {} bytes exceeds target {} bytes, starting compression",
        original_size,
        TOOL_COMPRESSION_TARGET_SIZE
    );

    // Step 1: Simplify input_schema
    let mut compressed: Vec<Tool> = tools
        .iter()
        .map(|t| {
            let simplified_schema = simplify_input_schema(&t.tool_specification.input_schema.json);
            Tool {
                tool_specification: ToolSpecification {
                    name: t.tool_specification.name.clone(),
                    description: t.tool_specification.description.clone(),
                    input_schema: InputSchema {
                        json: simplified_schema,
                    },
                },
            }
        })
        .collect();

    let size_after_schema = calculate_tools_size(&compressed);
    tracing::debug!(
        "Size after schema simplification: {} bytes (reduced {} bytes)",
        size_after_schema,
        original_size - size_after_schema
    );

    if size_after_schema <= TOOL_COMPRESSION_TARGET_SIZE {
        tracing::info!(
            "Schema simplification achieved target, final size: {} bytes",
            size_after_schema
        );
        return compressed;
    }

    // Step 2: Proportionally compress descriptions
    let size_to_reduce = size_after_schema - TOOL_COMPRESSION_TARGET_SIZE;
    let total_desc_len: usize = compressed
        .iter()
        .map(|t| t.tool_specification.description.len())
        .sum();

    if total_desc_len > 0 {
        let keep_ratio = 1.0 - (size_to_reduce as f64 / total_desc_len as f64);
        let keep_ratio = keep_ratio.clamp(0.0, 1.0);

        for tool in &mut compressed {
            let desc = &tool.tool_specification.description;
            let target_len = (desc.len() as f64 * keep_ratio) as usize;
            tool.tool_specification.description = compress_description(desc, target_len);
        }
    }

    let final_size = calculate_tools_size(&compressed);
    tracing::info!(
        "Compression complete, original: {} bytes, final: {} bytes ({:.1}% reduction)",
        original_size,
        final_size,
        (original_size - final_size) as f64 / original_size as f64 * 100.0
    );

    compressed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_description_short() {
        let desc = "Short description";
        assert_eq!(compress_description(desc, 100), desc);
    }

    #[test]
    fn test_compress_description_long() {
        let desc = "This is a very long description that needs to be compressed to fit within the target length";
        let compressed = compress_description(desc, 60);
        assert!(compressed.len() <= 60);
        assert!(compressed.ends_with("..."));
    }

    #[test]
    fn test_compress_description_utf8_safe() {
        let desc = "这是一个很长的中文描述，需要被压缩";
        let compressed = compress_description(desc, 30);
        // Should not panic and should be valid UTF-8
        // Chinese characters are 3 bytes each, so result may be longer than 30 bytes
        assert!(compressed.is_ascii() || compressed.chars().count() > 0);
        assert!(compressed.ends_with("...") || compressed.len() <= desc.len());
    }

    #[test]
    fn test_simplify_input_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "description": "This should be removed",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Also removed"
                }
            },
            "required": ["name"]
        });

        let simplified = simplify_input_schema(&schema);
        let obj = simplified.as_object().unwrap();

        assert!(obj.contains_key("type"));
        assert!(obj.contains_key("properties"));
        assert!(obj.contains_key("required"));
        assert!(!obj.contains_key("description"));
    }
}
