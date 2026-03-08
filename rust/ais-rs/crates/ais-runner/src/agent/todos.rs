use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Todo,
    InProgress,
    Done,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub required_facts: Vec<String>,
    #[serde(default)]
    pub produced_facts: Vec<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
    pub status: TodoStatus,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub segment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<TodoReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoSpec {
    pub title: String,
    #[serde(default)]
    pub required_facts: Vec<String>,
    #[serde(default)]
    pub produced_facts: Vec<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoReceipt {
    pub schema: String,
    pub todo_id: String,
    pub segment_id: String,
    pub status: String,
    #[serde(default)]
    pub paused_reason: Option<String>,
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub completed_node_ids: Vec<String>,
    #[serde(default)]
    pub tx_hashes: Vec<String>,
    #[serde(default)]
    pub event_types: Vec<String>,
    #[serde(default)]
    pub event_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TodoBoard {
    todos: Vec<TodoItem>,
    next_seq: u32,
}

impl TodoBoard {
    pub fn restore_or_bootstrap(runtime: &Value, intent: &str) -> Self {
        if let Some(progress) = runtime.pointer("/agent/todo_progress") {
            if let Ok(runtime_board) = serde_json::from_value::<TodoBoardRuntime>(progress.clone())
            {
                let mut board: TodoBoard = runtime_board.into();
                board.repair_next_seq();
                if !board.todos.is_empty() {
                    return board;
                }
            }
        }
        Self::bootstrap(intent)
    }

    pub fn bootstrap(intent: &str) -> Self {
        let mut board = Self {
            todos: Vec::new(),
            next_seq: 1,
        };
        board.push_todo(
            initial_title_from_intent(intent),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        board
    }

    pub fn ensure_current(&mut self) {
        if self.current().is_none() {
            self.push_todo(
                format!("Continue intent segment {}", self.next_seq),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
        }
    }

    pub fn current(&self) -> Option<&TodoItem> {
        self.current_index().map(|index| &self.todos[index])
    }

    pub fn current_todo_id(&self) -> Option<&str> {
        self.current().map(|item| item.id.as_str())
    }

    pub fn mark_current_in_progress(&mut self, summary: Option<&str>, segment_id: &str) {
        self.ensure_current();
        let Some(index) = self.current_index() else {
            return;
        };
        let item = &mut self.todos[index];
        item.status = TodoStatus::InProgress;
        item.blocked_reason = None;
        item.segment_id = Some(segment_id.to_string());
        item.receipt = None;
        if let Some(cleaned_summary) = normalize_summary(summary) {
            item.title = cleaned_summary;
        } else {
            item.title = format!("Execute segment {segment_id}");
        }
    }

    pub fn mark_current_todo(&mut self) {
        let Some(index) = self.current_index() else {
            return;
        };
        let item = &mut self.todos[index];
        item.status = TodoStatus::Todo;
        item.blocked_reason = None;
    }

    pub fn mark_current_done(&mut self) {
        let Some(index) = self.current_index() else {
            return;
        };
        let item = &mut self.todos[index];
        item.status = TodoStatus::Done;
        item.blocked_reason = None;
    }

    pub fn mark_current_blocked(&mut self, reason: impl Into<String>) {
        self.ensure_current();
        let Some(index) = self.current_index() else {
            return;
        };
        let item = &mut self.todos[index];
        item.status = TodoStatus::Blocked;
        item.blocked_reason = Some(reason.into());
    }

    pub fn open_follow_up_todo(&mut self) {
        self.push_todo(
            format!("Continue intent segment {}", self.next_seq),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
    }

    #[cfg(test)]
    pub fn intent_acceptance_complete_from_state_summary(
        &self,
        state_summary: Option<&Value>,
    ) -> bool {
        if self.current().is_some() {
            return false;
        }
        let acceptance_rules = self
            .todos
            .iter()
            .flat_map(|item| item.acceptance.iter().map(String::as_str))
            .collect::<Vec<_>>();
        if acceptance_rules.is_empty() {
            return false;
        }
        acceptance_rules
            .iter()
            .all(|rule| acceptance_rule_resolved_from_state_summary(state_summary, rule))
    }

    pub fn intent_acceptance_complete(
        &self,
        state_summary: Option<&super::state_summary::StateSummary>,
    ) -> bool {
        if self.current().is_some() {
            return false;
        }
        let acceptance_rules = self
            .todos
            .iter()
            .flat_map(|item| item.acceptance.iter().map(String::as_str))
            .collect::<Vec<_>>();
        if acceptance_rules.is_empty() {
            return false;
        }
        acceptance_rules
            .iter()
            .all(|rule| acceptance_rule_resolved(state_summary, rule))
    }

    pub fn record_receipt_for_todo(&mut self, todo_id: &str, receipt: TodoReceipt) -> bool {
        let Some(item) = self.todos.iter_mut().find(|item| item.id == todo_id) else {
            return false;
        };
        item.receipt = Some(receipt);
        true
    }

    pub fn replace_from_specs(&mut self, intent: &str, specs: &[TodoSpec]) -> usize {
        self.todos.clear();
        self.next_seq = 1;
        let mut seen_titles = BTreeSet::<String>::new();
        let mut rejected_placeholder_tail = 0usize;
        for spec in specs {
            let Some(title) = normalize_summary(Some(spec.title.as_str())) else {
                continue;
            };
            if is_placeholder_follow_up_title(title.as_str()) {
                rejected_placeholder_tail = rejected_placeholder_tail.saturating_add(1);
                continue;
            }
            let dedupe_key = title.to_lowercase();
            if !seen_titles.insert(dedupe_key) {
                continue;
            }
            self.push_todo(
                title,
                normalize_fact_keys(spec.required_facts.as_slice()),
                normalize_fact_keys(spec.produced_facts.as_slice()),
                normalize_acceptance(spec.acceptance.as_slice()),
            );
        }
        if self.todos.is_empty() {
            self.push_todo(
                initial_title_from_intent(intent),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
        }
        rejected_placeholder_tail
    }

    pub fn to_runtime_value(&self) -> Value {
        let (todo_count, in_progress_count, done_count, blocked_count) = self.status_counts();
        json!({
            "schema": "ais-agent-todo-progress/0.0.1",
            "current_todo": self.current(),
            "todos": self.todos,
            "progress": {
                "todo": todo_count,
                "in_progress": in_progress_count,
                "done": done_count,
                "blocked": blocked_count,
                "total": self.todos.len(),
            },
            "next_seq": self.next_seq,
        })
    }

    fn status_counts(&self) -> (usize, usize, usize, usize) {
        let mut todo_count = 0usize;
        let mut in_progress_count = 0usize;
        let mut done_count = 0usize;
        let mut blocked_count = 0usize;
        for item in &self.todos {
            match item.status {
                TodoStatus::Todo => todo_count = todo_count.saturating_add(1),
                TodoStatus::InProgress => in_progress_count = in_progress_count.saturating_add(1),
                TodoStatus::Done => done_count = done_count.saturating_add(1),
                TodoStatus::Blocked => blocked_count = blocked_count.saturating_add(1),
            }
        }
        (todo_count, in_progress_count, done_count, blocked_count)
    }

    fn current_index(&self) -> Option<usize> {
        self.todos
            .iter()
            .position(|item| item.status != TodoStatus::Done)
    }

    fn push_todo(
        &mut self,
        title: String,
        required_facts: Vec<String>,
        produced_facts: Vec<String>,
        acceptance: Vec<String>,
    ) {
        let id = format!("todo_{}", self.next_seq.max(1));
        self.next_seq = self.next_seq.max(1).saturating_add(1);
        self.todos.push(TodoItem {
            id,
            title,
            required_facts,
            produced_facts,
            acceptance: if acceptance.is_empty() {
                vec!["segment_compiled_and_executed".to_string()]
            } else {
                acceptance
            },
            status: TodoStatus::Todo,
            blocked_reason: None,
            segment_id: None,
            receipt: None,
        });
    }

    fn repair_next_seq(&mut self) {
        let max_seen = self
            .todos
            .iter()
            .filter_map(|item| {
                item.id
                    .strip_prefix("todo_")
                    .and_then(|suffix| suffix.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0);
        self.next_seq = self.next_seq.max(max_seen.saturating_add(1)).max(1);
    }
}

fn normalize_summary(summary: Option<&str>) -> Option<String> {
    let text = summary.map(str::trim).unwrap_or_default();
    if text.is_empty() {
        return None;
    }
    let mut shortened = String::new();
    for character in text.chars().take(96) {
        shortened.push(character);
    }
    if shortened.is_empty() {
        None
    } else {
        Some(shortened)
    }
}

fn normalize_fact_keys(keys: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::<String>::new();
    let mut normalized = Vec::<String>::new();
    for key in keys {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            normalized.push(trimmed.to_string());
        }
    }
    normalized
}

fn normalize_acceptance(rules: &[String]) -> Vec<String> {
    normalize_fact_keys(rules)
}

fn initial_title_from_intent(intent: &str) -> String {
    let compact = intent
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if compact.is_empty() {
        return "Execute intent segment 1".to_string();
    }
    let mut preview = String::new();
    for character in compact.chars().take(80) {
        preview.push(character);
    }
    format!("Execute intent: {preview}")
}

fn is_placeholder_follow_up_title(title: &str) -> bool {
    let normalized = title.trim().to_ascii_lowercase();
    normalized.starts_with("continue intent segment ")
}

#[cfg(test)]
fn acceptance_rule_resolved_from_state_summary(state_summary: Option<&Value>, rule: &str) -> bool {
    let Some(summary) = state_summary else {
        return false;
    };
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    if let Some(canonical_slot) = super::input_normalize::normalize_input_slot_key(rule) {
        let canonical_ref = format!("inputs.{canonical_slot}");
        if super::reference_inventory::ReferenceInventory::build(Some(summary))
            .input_refs()
            .iter()
            .any(|known| known == &canonical_ref)
        {
            return true;
        }
    }
    false
}

fn acceptance_rule_resolved(
    state_summary: Option<&super::state_summary::StateSummary>,
    rule: &str,
) -> bool {
    let Some(summary) = state_summary else {
        return false;
    };
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    if let Some(canonical_slot) = super::input_normalize::normalize_input_slot_key(rule) {
        let canonical_ref = format!("inputs.{canonical_slot}");
        if super::known_input_refs_from_typed_summary(Some(summary))
            .iter()
            .any(|known| known == &canonical_ref)
        {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct TodoBoardRuntime {
    todos: Vec<TodoItem>,
    next_seq: u32,
}

impl Default for TodoBoardRuntime {
    fn default() -> Self {
        Self {
            todos: Vec::new(),
            next_seq: 1,
        }
    }
}

impl From<TodoBoardRuntime> for TodoBoard {
    fn from(value: TodoBoardRuntime) -> Self {
        Self {
            todos: value.todos,
            next_seq: value.next_seq,
        }
    }
}

#[cfg(test)]
#[path = "tests/todos.rs"]
mod tests;
