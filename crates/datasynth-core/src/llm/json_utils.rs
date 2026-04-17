//! Utilities for extracting JSON fragments from noisy LLM output.

/// Extract the first balanced JSON object (`{...}`) from text.
pub fn extract_json_object(content: &str) -> Option<&str> {
    extract_balanced(content, '{', '}')
}

/// Extract the first balanced JSON array (`[...]`) from text.
pub fn extract_json_array(content: &str) -> Option<&str> {
    extract_balanced(content, '[', ']')
}

/// Extract the first balanced pair of delimiters from text.
fn extract_balanced(content: &str, open: char, close: char) -> Option<&str> {
    let start = content.find(open)?;
    let mut depth = 0i32;
    for (i, ch) in content[start..].char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(&content[start..start + i + 1]);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_object() {
        let input = r#"Here: {"a": 1, "b": {"c": 2}} done"#;
        let obj = extract_json_object(input).expect("extract_json_object returned None");
        assert!(obj.starts_with('{') && obj.ends_with('}'));
        assert!(obj.contains("\"c\": 2"));
    }

    #[test]
    fn test_extract_json_array() {
        let input = r#"Result: [{"x": 1}, {"y": [2, 3]}] end"#;
        let arr = extract_json_array(input).expect("extract_json_array returned None");
        assert!(arr.starts_with('[') && arr.ends_with(']'));
    }

    #[test]
    fn test_no_match() {
        assert!(extract_json_object("no json here").is_none());
        assert!(extract_json_array("no json here").is_none());
    }

    #[test]
    fn test_unbalanced() {
        assert!(extract_json_object("{unclosed").is_none());
        assert!(extract_json_array("[unclosed").is_none());
    }
}
