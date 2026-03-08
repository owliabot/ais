use ais_llm::{LlmMessage, MessageRole};

pub(super) struct MessageCompactorConfig {
    pub keep_recent_rounds: u8,
    pub messages_token_budget: u64,
}

impl Default for MessageCompactorConfig {
    fn default() -> Self {
        Self {
            keep_recent_rounds: 3,
            messages_token_budget: 0,
        }
    }
}

pub(super) struct MessageCompactor {
    config: MessageCompactorConfig,
}

impl MessageCompactor {
    pub fn new(config: MessageCompactorConfig) -> Self {
        Self { config }
    }

    pub fn should_compact(&self, messages: &[LlmMessage]) -> bool {
        if self.config.messages_token_budget == 0 {
            return false;
        }
        let rounds = count_rounds(messages);
        if rounds <= self.config.keep_recent_rounds as usize + 1 {
            return false;
        }
        let total_tokens = estimate_messages_tokens(messages);
        total_tokens > self.config.messages_token_budget
    }

    pub fn compact(&self, messages: &mut Vec<LlmMessage>) {
        let round_boundaries = find_round_boundaries(messages);
        let total_rounds = round_boundaries.len();
        let keep = self.config.keep_recent_rounds as usize;
        if total_rounds <= keep + 1 {
            return;
        }
        let compress_up_to = total_rounds.saturating_sub(keep);
        let compress_end_idx = round_boundaries[compress_up_to];
        let preamble_end = find_preamble_end(messages);
        if compress_end_idx <= preamble_end {
            return;
        }

        let summary = build_compressed_summary(&messages[preamble_end..compress_end_idx]);
        let kept_tail: Vec<LlmMessage> = messages[compress_end_idx..].to_vec();

        messages.truncate(preamble_end);
        messages.push(LlmMessage {
            role: MessageRole::User,
            content: Some(summary),
            tool_name: None,
            tool_call_id: None,
            tool_calls: vec![],
        });
        messages.extend(kept_tail);
    }
}

fn find_preamble_end(messages: &[LlmMessage]) -> usize {
    let mut idx = 0;
    for msg in messages {
        match msg.role {
            MessageRole::System | MessageRole::User if idx < 2 => idx += 1,
            _ => break,
        }
    }
    idx.min(messages.len())
}

fn find_round_boundaries(messages: &[LlmMessage]) -> Vec<usize> {
    let preamble_end = find_preamble_end(messages);
    let mut boundaries = vec![];
    let mut i = preamble_end;
    while i < messages.len() {
        if messages[i].role == MessageRole::Assistant {
            boundaries.push(i);
            i += 1;
            while i < messages.len() && messages[i].role != MessageRole::Assistant {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    boundaries
}

fn count_rounds(messages: &[LlmMessage]) -> usize {
    find_round_boundaries(messages).len()
}

fn estimate_messages_tokens(messages: &[LlmMessage]) -> u64 {
    messages
        .iter()
        .map(|m| {
            let content_tokens = m
                .content
                .as_ref()
                .map(|c| super::token_count::count_tokens(c))
                .unwrap_or(0);
            let tool_calls_tokens: usize = m
                .tool_calls
                .iter()
                .map(|tc| {
                    super::token_count::count_tokens(&tc.name)
                        + super::token_count::count_tokens(&tc.arguments.to_string())
                })
                .sum();
            content_tokens + tool_calls_tokens + 4 // ~4 tokens overhead per message
        })
        .sum::<usize>() as u64
}

fn build_compressed_summary(history: &[LlmMessage]) -> String {
    let mut tools_called: Vec<String> = vec![];
    let mut tool_results_count = 0u32;
    let mut rounds_compressed = 0u32;

    for msg in history {
        match msg.role {
            MessageRole::Assistant => {
                rounds_compressed += 1;
                for call in &msg.tool_calls {
                    if !tools_called.contains(&call.name) {
                        tools_called.push(call.name.clone());
                    }
                }
            }
            MessageRole::Tool => {
                tool_results_count += 1;
            }
            _ => {}
        }
    }

    let tools_str = if tools_called.is_empty() {
        "none".to_string()
    } else {
        tools_called.join(", ")
    };

    format!(
        "{{\"compressed_history\":true,\"rounds_compressed\":{rounds_compressed},\"tool_results\":{tool_results_count},\"tools_called\":\"{tools_str}\"}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ais_llm::ToolCall;
    use serde_json::json;

    fn system_msg(content: &str) -> LlmMessage {
        LlmMessage {
            role: MessageRole::System,
            content: Some(content.to_string()),
            tool_name: None,
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    fn user_msg(content: &str) -> LlmMessage {
        LlmMessage {
            role: MessageRole::User,
            content: Some(content.to_string()),
            tool_name: None,
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    fn assistant_msg(tool_names: &[&str]) -> LlmMessage {
        LlmMessage {
            role: MessageRole::Assistant,
            content: None,
            tool_name: None,
            tool_call_id: None,
            tool_calls: tool_names
                .iter()
                .enumerate()
                .map(|(i, name)| ToolCall {
                    id: format!("call_{i}"),
                    name: name.to_string(),
                    arguments: json!({}),
                })
                .collect(),
        }
    }

    fn tool_msg(name: &str, content: &str) -> LlmMessage {
        LlmMessage {
            role: MessageRole::Tool,
            content: Some(content.to_string()),
            tool_name: Some(name.to_string()),
            tool_call_id: Some("call_0".to_string()),
            tool_calls: vec![],
        }
    }

    fn build_messages(rounds: usize) -> Vec<LlmMessage> {
        let mut msgs = vec![
            system_msg("system prompt"),
            user_msg("user prompt with state_summary"),
        ];
        for r in 0..rounds {
            msgs.push(assistant_msg(&["catalog.discover"]));
            msgs.push(tool_msg("catalog.discover", &format!("result_{r}")));
        }
        msgs
    }

    #[test]
    fn no_compact_within_budget() {
        let msgs = build_messages(3);
        let compactor = MessageCompactor::new(MessageCompactorConfig {
            keep_recent_rounds: 2,
            messages_token_budget: 100_000,
        });
        assert!(!compactor.should_compact(&msgs));
    }

    #[test]
    fn no_compact_few_rounds() {
        let msgs = build_messages(2);
        let compactor = MessageCompactor::new(MessageCompactorConfig {
            keep_recent_rounds: 2,
            messages_token_budget: 1,
        });
        assert!(!compactor.should_compact(&msgs));
    }

    #[test]
    fn compact_reduces_messages() {
        let mut msgs = build_messages(8);
        let original_len = msgs.len(); // 2 + 8*2 = 18
        let compactor = MessageCompactor::new(MessageCompactorConfig {
            keep_recent_rounds: 2,
            messages_token_budget: 1,
        });
        assert!(compactor.should_compact(&msgs));
        compactor.compact(&mut msgs);

        // preamble(2) + summary(1) + keep_recent_rounds(2) * 2 msgs each = 7
        assert_eq!(msgs.len(), 7);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert_eq!(msgs[1].role, MessageRole::User);
        assert_eq!(msgs[2].role, MessageRole::User); // compressed summary
        assert!(msgs[2]
            .content
            .as_ref()
            .unwrap()
            .contains("compressed_history"));
        assert_eq!(msgs[3].role, MessageRole::Assistant);
        assert!(original_len > msgs.len());
    }

    #[test]
    fn compact_preserves_recent_rounds() {
        let mut msgs = build_messages(6);
        let compactor = MessageCompactor::new(MessageCompactorConfig {
            keep_recent_rounds: 3,
            messages_token_budget: 1,
        });
        compactor.compact(&mut msgs);

        // preamble(2) + summary(1) + 3 rounds * 2 = 9
        assert_eq!(msgs.len(), 9);
        let last_tool = msgs.last().unwrap();
        assert_eq!(last_tool.role, MessageRole::Tool);
        assert!(last_tool.content.as_ref().unwrap().contains("result_5"));
    }

    #[test]
    fn summary_captures_tools() {
        let history = vec![
            assistant_msg(&["catalog.discover", "catalog.discover"]),
            tool_msg("catalog.discover", "res1"),
            tool_msg("catalog.discover", "res2"),
            assistant_msg(&["get_candidate_detail"]),
            tool_msg("get_candidate_detail", "res3"),
        ];
        let summary = build_compressed_summary(&history);
        assert!(summary.contains("catalog.discover"));
        assert!(summary.contains("catalog.discover"));
        assert!(summary.contains("get_candidate_detail"));
        assert!(summary.contains("\"rounds_compressed\":2"));
        assert!(summary.contains("\"tool_results\":3"));
    }

    #[test]
    fn disabled_when_budget_zero() {
        let msgs = build_messages(20);
        let compactor = MessageCompactor::new(MessageCompactorConfig {
            keep_recent_rounds: 2,
            messages_token_budget: 0,
        });
        assert!(!compactor.should_compact(&msgs));
    }

    #[test]
    fn round_boundary_detection() {
        let msgs = build_messages(4);
        let boundaries = find_round_boundaries(&msgs);
        assert_eq!(boundaries.len(), 4);
        assert_eq!(boundaries[0], 2); // first assistant after preamble
        assert_eq!(boundaries[1], 4);
        assert_eq!(boundaries[2], 6);
        assert_eq!(boundaries[3], 8);
    }

    #[test]
    fn compact_with_user_hints() {
        let mut msgs = vec![
            system_msg("system"),
            user_msg("user"),
            assistant_msg(&["catalog.discover"]),
            tool_msg("catalog.discover", "res0"),
            user_msg("hint0"),
            assistant_msg(&["catalog.discover"]),
            tool_msg("catalog.discover", "res1"),
            assistant_msg(&["get_candidate_detail"]),
            tool_msg("get_candidate_detail", "res2"),
            assistant_msg(&["guide.get"]),
            tool_msg("guide.get", "res3"),
        ];
        let compactor = MessageCompactor::new(MessageCompactorConfig {
            keep_recent_rounds: 2,
            messages_token_budget: 1,
        });
        compactor.compact(&mut msgs);

        // preamble(2) + summary(1) + last 2 rounds (2*2=4) = 7
        assert_eq!(msgs.len(), 7);
        assert!(msgs[2]
            .content
            .as_ref()
            .unwrap()
            .contains("compressed_history"));
        assert_eq!(msgs[3].role, MessageRole::Assistant);
    }
}
