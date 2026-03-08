use serde::Serialize;
use serde_json::Value;

pub(super) const ESTIMATOR_SOURCE: &str = "tiktoken(o200k_base)";

pub(super) fn count_tokens(text: &str) -> usize {
    tiktoken_rs::o200k_base_singleton()
        .encode_with_special_tokens(text)
        .len()
}

pub(super) fn count_tokens_json(value: &Value) -> u64 {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    count_tokens(&encoded) as u64
}

pub(super) fn count_tokens_serializable<T: Serialize>(value: &T) -> u64 {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    count_tokens(&encoded) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_tokens_basic() {
        let n = count_tokens("hello world");
        assert!(n >= 1 && n <= 4, "expected 1-4 tokens, got {n}");
    }

    #[test]
    fn count_tokens_json_object() {
        let val = serde_json::json!({"key": "value", "num": 42});
        let n = count_tokens_json(&val);
        assert!(n >= 3 && n <= 20, "expected 3-20 tokens, got {n}");
    }

    #[test]
    fn count_tokens_serializable_struct() {
        #[derive(Serialize)]
        struct Demo {
            name: String,
            count: u64,
        }
        let demo = Demo {
            name: "test".to_string(),
            count: 100,
        };
        let n = count_tokens_serializable(&demo);
        assert!(n >= 3 && n <= 20, "expected 3-20 tokens, got {n}");
    }

    #[test]
    fn estimator_source_label() {
        assert_eq!(ESTIMATOR_SOURCE, "tiktoken(o200k_base)");
    }
}
