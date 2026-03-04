use crate::error::RunnerError;
use ais_core::{stable_hash_hex, StableJsonOptions};
use ais_llm::{CompleteWithToolsRequest, LlmMessage, LlmProvider, MessageRole, ToolCall, ToolSpec};
use ais_schema::{
    get_json_schema,
    versions::{SCHEMA_AGENT_PLANNING_TOOLS_0_1_0, SCHEMA_PLAN_SKETCH_0_1_0},
};
use ais_sdk::documents::PlanSketchSegment;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use super::budget::{compact_json_with_options, JsonBudgetOptions};
use super::candidates::{CandidateContext, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT};
use super::context::budget_policy::ToolMemoryBudgetPolicy;
use super::planning_memory::{
    PlanningMemory, PlanningMemoryBudget, ToolMemoryProjectionCandidates,
};
use super::sanitize::sanitize_for_llm_payload;
use super::todos::TodoSpec;
use super::tools::decode::{normalize_tool_args_for_validation, phase_from_finalize_tool};
use super::tools::dispatch::{DecodedSegmentedToolCall, PlannerToolOutput};
use super::tools::phase_policy::{
    ensure_tool_allowed_for_phase, phase_name, validate_tool_calls_for_phase,
};

pub trait SegmentedIntentPlanner {
    fn begin_session(
        &mut self,
        request: SegmentBeginRequest,
    ) -> Result<SegmentPlanningSession, RunnerError>;

    fn propose_segment(
        &mut self,
        request: SegmentPlanningRequest,
    ) -> Result<SegmentDraft, RunnerError>;

    fn propose_todos(&mut self, request: TodoPlanningRequest) -> Result<TodoDraft, RunnerError>;

    fn ground_intent(
        &mut self,
        request: IntentGroundingRequest,
    ) -> Result<IntentGroundingDraft, RunnerError>;

    fn revise_segment(
        &mut self,
        request: SegmentPlanningRequest,
    ) -> Result<SegmentDraft, RunnerError>;
}

#[derive(Debug, Clone)]
pub struct SegmentBeginRequest {
    pub intent: String,
    pub snapshot_hash: String,
    pub pack_snapshot_hash: String,
    pub catalog_hash: String,
    pub chain_scope: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SegmentPlanningRequest {
    pub intent: String,
    pub session: SegmentPlanningSession,
    pub state_summary: Option<Value>,
    pub previous_error: Option<Value>,
    pub last_segment: Option<PlanSketchSegment>,
}

#[derive(Debug, Clone)]
pub struct TodoPlanningRequest {
    pub intent: String,
    pub session: SegmentPlanningSession,
    pub state_summary: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct IntentGroundingRequest {
    pub intent: String,
    pub session: SegmentPlanningSession,
    pub state_summary: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct SegmentPlanningSession {
    pub session_id: String,
    pub snapshot_hash: String,
    pub cursor: String,
    pub max_rounds: u8,
    pub max_segments: u8,
}

#[derive(Debug, Clone)]
pub enum SegmentDraft {
    Proposed {
        summary: Option<String>,
        segment: PlanSketchSegment,
        cursor_next: String,
        done: bool,
        issues: Vec<Value>,
    },
    Unavailable {
        reason_code: String,
        message: Option<String>,
        done: bool,
        issues: Vec<Value>,
        questions: Vec<Value>,
    },
    Invalid {
        reason_code: String,
        message: Option<String>,
        done: bool,
        issues: Vec<Value>,
    },
}

#[derive(Debug, Clone)]
pub enum TodoDraft {
    Proposed {
        summary: Option<String>,
        todos: Vec<TodoSpec>,
        issues: Vec<Value>,
    },
    Unavailable {
        reason_code: String,
        message: Option<String>,
        issues: Vec<Value>,
        questions: Vec<Value>,
    },
    Invalid {
        reason_code: String,
        message: Option<String>,
        issues: Vec<Value>,
    },
}

#[derive(Debug, Clone)]
pub enum IntentGroundingDraft {
    Proposed {
        summary: Option<String>,
        ready_for_todos: bool,
        resolved_inputs: BTreeMap<String, Value>,
        intent_facts: BTreeMap<String, Value>,
        confidence: BTreeMap<String, u8>,
        issues: Vec<Value>,
        questions: Vec<Value>,
    },
    Unavailable {
        reason_code: String,
        message: Option<String>,
        issues: Vec<Value>,
        questions: Vec<Value>,
    },
    Invalid {
        reason_code: String,
        message: Option<String>,
        issues: Vec<Value>,
    },
}

pub struct LlmSegmentedIntentPlanner<P> {
    provider: P,
    candidate_context: Option<CandidateContext>,
    planning_memory: PlanningMemory,
    diagnostics_tracker: PlannerDiagnosticsTracker,
    last_failed_finalize: Option<Value>,
    begin_context: Option<PlannerBeginContext>,
    prompt_builder: SegmentedPromptContextBuilder,
    prompt_overrides: SegmentedPromptOverrides,
    max_tool_rounds: u8,
    verbose_llm: bool,
    usage_tracker: PlannerLlmUsageTracker,
    llm_transcript: Option<LlmTranscriptSink>,
}

#[derive(Debug, Clone)]
struct PlannerBeginContext {
    pack_snapshot_hash: String,
    chain_scope: Vec<String>,
}

#[derive(Debug, Clone)]
struct LlmTranscriptSink {
    path: PathBuf,
    append: bool,
    initialized: bool,
}

#[derive(Debug, Clone)]
pub(super) struct SegmentCheckContext {
    pub(super) intent: String,
    pub(super) session_id: String,
    pub(super) cursor: String,
    pub(super) pack_snapshot_hash: String,
    pub(super) chain_scope: Vec<String>,
    pub(super) known_input_refs: Vec<String>,
    pub(super) grounding_fact_keys: Vec<String>,
    pub(super) current_todo: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlannerRoundPhase {
    Begin,
    GroundIntent,
    ProposeTodos,
    ProposeSegment,
    ReviseSegment,
}

const SEGMENTED_PROMPT_VERSION: &str = "aisrs-segmented-planner-v2";
pub(crate) const DEFAULT_SEGMENTED_MAX_TOOL_ROUNDS: u8 = 24;
const REPEATED_PLAN_CHECK_FAILURE_THRESHOLD: u64 = 3;
const FINALIZE_SCHEMA_REPAIR_ATTEMPT_LIMIT: u8 = 2;
const NON_FINALIZE_TOOL_SCHEMA_REPAIR_ATTEMPT_LIMIT: u8 = 2;
const NO_TOOLCALL_RETRY_ATTEMPT_LIMIT: u8 = 2;
const ADJUDICATE_MAX_TOOL_ROUNDS: u8 = 3;
const ADJUDICATE_EMPTY_SEARCH_STREAK_LIMIT: u64 = 2;
const LLM_CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const CONTEXT_SOFT_LIMIT_NUMERATOR: u64 = 9;
const CONTEXT_SOFT_LIMIT_DENOMINATOR: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannerLlmCallUsage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cumulative_input_tokens: u64,
    cumulative_output_tokens: u64,
    cumulative_total_tokens: u64,
    context_limit_tokens: Option<u64>,
    context_soft_limit_tokens: Option<u64>,
    context_remaining_tokens: Option<u64>,
    estimated: bool,
    source: &'static str,
}

#[derive(Debug, Clone, Default)]
struct PlannerLlmUsageTracker {
    calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    estimated_calls: u64,
    context_limit_tokens: Option<u64>,
    latest_input_tokens: u64,
    latest_total_tokens: u64,
}

#[derive(Debug, Clone, Default)]
struct PlannerDiagnosticsTracker {
    total_tool_calls: u64,
    duplicate_tool_calls: u64,
    tool_call_count_by_tool: BTreeMap<String, u64>,
    tool_result_count_by_tool: BTreeMap<String, u64>,
    memory_hits_by_tool: BTreeMap<String, u64>,
    phase_round_count: BTreeMap<String, u64>,
    memory_projection_budget_tokens: u64,
    memory_projection_estimated_tokens: u64,
    finalize_schema_repair_attempts_total: u64,
    finalize_schema_repair_exhausted_total: u64,
    finalize_schema_repair_by_sub_reason: BTreeMap<String, u64>,
    no_toolcall_retries_total: u64,
    no_toolcall_retries_exhausted_total: u64,
    empty_search_streak_max: u64,
    parallel_batches_total: u64,
    parallel_calls_total: u64,
    parallel_failures_total: u64,
    parallel_partial_success_total: u64,
    tool_exec_total: u64,
    tool_exec_success: u64,
    tool_exec_error: u64,
    tool_exec_cached_hit: u64,
    tool_exec_parallel: u64,
    tool_exec_sequential: u64,
    tool_exec_blocked_finalize: u64,
    tool_exec_repair_retry: u64,
    tool_exec_repair_exhausted: u64,
    tool_exec_count_by_tool: BTreeMap<String, u64>,
    tool_exec_error_by_tool: BTreeMap<String, u64>,
    tool_exec_latency_sum_ms_by_tool: BTreeMap<String, u64>,
    tool_exec_latency_max_ms_by_tool: BTreeMap<String, u64>,
    seen_tool_call_keys: BTreeSet<String>,
}

impl PlannerDiagnosticsTracker {
    fn observe_phase_round(&mut self, phase: PlannerRoundPhase) {
        let key = phase_name(phase).to_string();
        let entry = self.phase_round_count.entry(key).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    fn observe_tool_call(&mut self, tool_name: &str, dedupe_key: Option<String>) -> bool {
        self.total_tool_calls = self.total_tool_calls.saturating_add(1);
        let entry = self
            .tool_call_count_by_tool
            .entry(tool_name.to_string())
            .or_insert(0);
        *entry = entry.saturating_add(1);
        let Some(key) = dedupe_key else {
            return false;
        };
        if !self.seen_tool_call_keys.insert(key) {
            self.duplicate_tool_calls = self.duplicate_tool_calls.saturating_add(1);
            return true;
        }
        false
    }

    fn observe_tool_result(&mut self, tool_name: &str, cached: bool) {
        let total = self
            .tool_result_count_by_tool
            .entry(tool_name.to_string())
            .or_insert(0);
        *total = total.saturating_add(1);
        if cached {
            let hits = self
                .memory_hits_by_tool
                .entry(tool_name.to_string())
                .or_insert(0);
            *hits = hits.saturating_add(1);
        }
    }

    fn observe_empty_search_streak(&mut self, streak: u64) {
        if streak > self.empty_search_streak_max {
            self.empty_search_streak_max = streak;
        }
    }

    fn observe_finalize_schema_repair_attempt(&mut self, sub_reason_code: &str) {
        self.finalize_schema_repair_attempts_total =
            self.finalize_schema_repair_attempts_total.saturating_add(1);
        let entry = self
            .finalize_schema_repair_by_sub_reason
            .entry(sub_reason_code.to_string())
            .or_insert(0);
        *entry = entry.saturating_add(1);
    }

    fn observe_tool_memory_projection(
        &mut self,
        budget_tokens: usize,
        estimated_tokens: Option<u64>,
    ) {
        self.memory_projection_budget_tokens = u64::try_from(budget_tokens).unwrap_or(u64::MAX);
        self.memory_projection_estimated_tokens = estimated_tokens.unwrap_or(0);
    }

    fn observe_finalize_schema_repair_exhausted(&mut self) {
        self.finalize_schema_repair_exhausted_total = self
            .finalize_schema_repair_exhausted_total
            .saturating_add(1);
    }

    fn observe_no_toolcall_retry(&mut self) {
        self.no_toolcall_retries_total = self.no_toolcall_retries_total.saturating_add(1);
    }

    fn observe_no_toolcall_retry_exhausted(&mut self) {
        self.no_toolcall_retries_exhausted_total =
            self.no_toolcall_retries_exhausted_total.saturating_add(1);
    }

    fn observe_parallel_batch(&mut self, calls: u64) {
        self.parallel_batches_total = self.parallel_batches_total.saturating_add(1);
        self.parallel_calls_total = self.parallel_calls_total.saturating_add(calls);
    }

    fn observe_parallel_partial_success(&mut self) {
        self.parallel_partial_success_total = self.parallel_partial_success_total.saturating_add(1);
    }

    fn observe_tool_exec_end(
        &mut self,
        tool_name: &str,
        mode: &'static str,
        status: &'static str,
        latency_ms: u64,
    ) {
        self.tool_exec_total = self.tool_exec_total.saturating_add(1);
        let total = self
            .tool_exec_count_by_tool
            .entry(tool_name.to_string())
            .or_insert(0);
        *total = total.saturating_add(1);
        if mode == "parallel" {
            self.tool_exec_parallel = self.tool_exec_parallel.saturating_add(1);
        } else {
            self.tool_exec_sequential = self.tool_exec_sequential.saturating_add(1);
        }
        match status {
            "success" => self.tool_exec_success = self.tool_exec_success.saturating_add(1),
            "cached_hit" => {
                self.tool_exec_cached_hit = self.tool_exec_cached_hit.saturating_add(1);
            }
            "blocked_finalize" => {
                self.tool_exec_blocked_finalize = self.tool_exec_blocked_finalize.saturating_add(1);
            }
            _ => {
                self.tool_exec_error = self.tool_exec_error.saturating_add(1);
                let errors = self
                    .tool_exec_error_by_tool
                    .entry(tool_name.to_string())
                    .or_insert(0);
                *errors = errors.saturating_add(1);
                if mode == "parallel" {
                    self.parallel_failures_total = self.parallel_failures_total.saturating_add(1);
                }
            }
        }
        let latency_sum = self
            .tool_exec_latency_sum_ms_by_tool
            .entry(tool_name.to_string())
            .or_insert(0);
        *latency_sum = latency_sum.saturating_add(latency_ms);
        let latency_max = self
            .tool_exec_latency_max_ms_by_tool
            .entry(tool_name.to_string())
            .or_insert(0);
        if latency_ms > *latency_max {
            *latency_max = latency_ms;
        }
    }

    fn observe_tool_exec_retry(&mut self, exhausted: bool) {
        if exhausted {
            self.tool_exec_repair_exhausted = self.tool_exec_repair_exhausted.saturating_add(1);
        } else {
            self.tool_exec_repair_retry = self.tool_exec_repair_retry.saturating_add(1);
        }
    }

    fn duplicate_ratio_bps(&self) -> u64 {
        if self.total_tool_calls == 0 {
            return 0;
        }
        self.duplicate_tool_calls.saturating_mul(10_000) / self.total_tool_calls
    }

    fn discovery_ratio_bps(&self) -> u64 {
        if self.total_tool_calls == 0 {
            return 0;
        }
        let discovery_calls = [
            "list_candidates",
            "catalog.search",
            "get_candidate_detail",
            "guide.get",
        ]
        .iter()
        .map(|tool| {
            self.tool_call_count_by_tool
                .get(*tool)
                .copied()
                .unwrap_or(0)
        })
        .sum::<u64>();
        discovery_calls.saturating_mul(10_000) / self.total_tool_calls
    }

    fn memory_hit_rate_by_tool_value(&self) -> Value {
        let mut output = serde_json::Map::<String, Value>::new();
        for (tool_name, total) in &self.tool_result_count_by_tool {
            let hits = self
                .memory_hits_by_tool
                .get(tool_name)
                .copied()
                .unwrap_or(0);
            let rate_bps = if *total == 0 {
                0
            } else {
                hits.saturating_mul(10_000) / *total
            };
            output.insert(
                tool_name.clone(),
                json!({
                    "hits": hits,
                    "total": total,
                    "rate_bps": rate_bps,
                }),
            );
        }
        Value::Object(output)
    }

    fn to_value(&self) -> Value {
        let ratio_bps = self.duplicate_ratio_bps();
        let discovery_ratio_bps = self.discovery_ratio_bps();
        json!({
            "tool_calls_total": self.total_tool_calls,
            "tool_calls_duplicate": self.duplicate_tool_calls,
            "duplicate_tool_call_ratio_bps": ratio_bps,
            "duplicate_tool_call_ratio": (ratio_bps as f64) / 10_000.0_f64,
            "discovery_tool_call_ratio_bps": discovery_ratio_bps,
            "discovery_tool_call_ratio": (discovery_ratio_bps as f64) / 10_000.0_f64,
            "tool_call_count_by_tool": self.tool_call_count_by_tool,
            "memory_hit_rate_by_tool": self.memory_hit_rate_by_tool_value(),
            "phase_round_count": self.phase_round_count,
            "finalize_schema_repair_attempts_total": self.finalize_schema_repair_attempts_total,
            "finalize_schema_repair_exhausted_total": self.finalize_schema_repair_exhausted_total,
            "finalize_schema_repair_by_sub_reason": self.finalize_schema_repair_by_sub_reason,
            "no_toolcall_retries_total": self.no_toolcall_retries_total,
            "no_toolcall_retries_exhausted_total": self.no_toolcall_retries_exhausted_total,
            "memory_projection_budget_tokens": self.memory_projection_budget_tokens,
            "memory_projection_estimated_tokens": self.memory_projection_estimated_tokens,
            "empty_search_streak_max": self.empty_search_streak_max,
            "parallel_batches_total": self.parallel_batches_total,
            "parallel_calls_total": self.parallel_calls_total,
            "parallel_failures_total": self.parallel_failures_total,
            "parallel_partial_success_total": self.parallel_partial_success_total,
            "tool_exec": {
                "total": self.tool_exec_total,
                "success": self.tool_exec_success,
                "error": self.tool_exec_error,
                "cached_hit": self.tool_exec_cached_hit,
                "parallel": self.tool_exec_parallel,
                "sequential": self.tool_exec_sequential,
                "blocked_finalize": self.tool_exec_blocked_finalize,
                "repair_retry": self.tool_exec_repair_retry,
                "repair_exhausted": self.tool_exec_repair_exhausted,
                "count_by_tool": self.tool_exec_count_by_tool,
                "error_by_tool": self.tool_exec_error_by_tool,
                "latency_sum_ms_by_tool": self.tool_exec_latency_sum_ms_by_tool,
                "latency_max_ms_by_tool": self.tool_exec_latency_max_ms_by_tool,
            }
        })
    }
}

#[derive(Debug, Clone, Default)]
struct CatalogSearchLoopGuard {
    current_streak: u64,
    max_streak: u64,
    previous_signature: Option<String>,
}

impl CatalogSearchLoopGuard {
    fn observe_empty(&mut self, signature: Option<String>) -> bool {
        if let (Some(previous), Some(current)) =
            (self.previous_signature.as_ref(), signature.as_ref())
        {
            if previous == current {
                self.current_streak = self.current_streak.saturating_add(1);
            } else {
                self.current_streak = 1;
            }
        } else {
            self.current_streak = 1;
        }
        self.previous_signature = signature;
        if self.current_streak > self.max_streak {
            self.max_streak = self.current_streak;
        }
        // Emit loop-guard hint only once when an empty-search streak first
        // reaches the threshold; avoid injecting the same hint every round.
        self.current_streak == 2
    }

    fn observe_non_empty(&mut self) {
        self.current_streak = 0;
        self.previous_signature = None;
    }

    fn max_streak(&self) -> u64 {
        self.max_streak
    }
}

#[derive(Debug, Clone, Default)]
struct PlanCheckFailureLoopGuard {
    current_streak: u64,
    previous_signature: Option<String>,
}

impl PlanCheckFailureLoopGuard {
    fn observe(&mut self, signature: Option<String>) -> bool {
        let Some(current) = signature else {
            self.current_streak = 0;
            self.previous_signature = None;
            return false;
        };
        if self
            .previous_signature
            .as_ref()
            .is_some_and(|prev| prev == &current)
        {
            self.current_streak = self.current_streak.saturating_add(1);
        } else {
            self.current_streak = 1;
        }
        self.previous_signature = Some(current);
        self.current_streak >= REPEATED_PLAN_CHECK_FAILURE_THRESHOLD
    }

    fn streak(&self) -> u64 {
        self.current_streak
    }
}

#[derive(Debug, Clone, Default)]
struct RoundContextSignal {
    pressure_mode: Option<String>,
    compressed: bool,
    adjudicate_mode: bool,
}

impl PlannerLlmUsageTracker {
    fn with_context_limit_tokens(mut self, context_limit_tokens: Option<u64>) -> Self {
        self.context_limit_tokens = context_limit_tokens;
        self
    }

    fn record_estimated(
        &mut self,
        request: &CompleteWithToolsRequest,
        response: &ais_llm::CompleteWithToolsResponse,
    ) -> PlannerLlmCallUsage {
        let input_tokens = estimate_tokens_from_json(request);
        let output_tokens = estimate_tokens_from_json(response);
        let total_tokens = input_tokens.saturating_add(output_tokens);
        self.calls = self.calls.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(total_tokens);
        self.estimated_calls = self.estimated_calls.saturating_add(1);
        self.latest_input_tokens = input_tokens;
        self.latest_total_tokens = total_tokens;
        let context_soft_limit_tokens = self.context_limit_tokens.map(context_soft_limit_tokens);
        // Context remaining should represent current request headroom, not session cumulative usage.
        let context_remaining_tokens =
            context_soft_limit_tokens.map(|soft_limit| soft_limit.saturating_sub(input_tokens));
        PlannerLlmCallUsage {
            input_tokens,
            output_tokens,
            total_tokens,
            cumulative_input_tokens: self.input_tokens,
            cumulative_output_tokens: self.output_tokens,
            cumulative_total_tokens: self.total_tokens,
            context_limit_tokens: self.context_limit_tokens,
            context_soft_limit_tokens,
            context_remaining_tokens,
            estimated: true,
            source: "estimated(chars_div_4)",
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "schema": "ais-agent-llm-usage/0.0.1",
            "calls": self.calls,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "estimated_calls": self.estimated_calls,
            "source": "estimated(chars_div_4)",
            "context_limit_tokens": self.context_limit_tokens,
            "context_soft_limit_tokens": self.context_limit_tokens.map(context_soft_limit_tokens),
            "context_window_input_tokens": self.latest_input_tokens,
            "context_window_total_tokens": self.latest_total_tokens,
            "context_remaining_tokens": self.context_limit_tokens
                .map(context_soft_limit_tokens)
                .map(|soft_limit| soft_limit.saturating_sub(self.latest_input_tokens)),
        })
    }
}

fn context_soft_limit_tokens(context_limit_tokens: u64) -> u64 {
    context_limit_tokens.saturating_mul(CONTEXT_SOFT_LIMIT_NUMERATOR)
        / CONTEXT_SOFT_LIMIT_DENOMINATOR
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptRenderOutput {
    version: String,
    hash: String,
    prompt: String,
}

#[derive(Debug, Clone)]
struct SegmentedPromptContextBuilder {
    base_rules: Vec<String>,
    phase_rules_begin: Vec<String>,
    phase_rules_grounding: Vec<String>,
    phase_rules_todos: Vec<String>,
    phase_rules_propose: Vec<String>,
    phase_rules_revise: Vec<String>,
    contracts_summary: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SegmentedPromptOverrides {
    pub base_rules: Option<Vec<String>>,
    pub phase_rules_begin: Option<Vec<String>>,
    pub phase_rules_grounding: Option<Vec<String>>,
    pub phase_rules_todos: Option<Vec<String>>,
    pub phase_rules_propose: Option<Vec<String>>,
    pub phase_rules_revise: Option<Vec<String>>,
    pub contracts_summary: Option<Vec<String>>,
    pub begin_payload_patch: Option<Value>,
    pub grounding_payload_patch: Option<Value>,
    pub todos_payload_patch: Option<Value>,
    pub segment_payload_patch: Option<Value>,
}

impl Default for SegmentedPromptContextBuilder {
    fn default() -> Self {
        Self {
            base_rules: vec![
                "Tool-calling only.",
                "Emit schema-typed JSON only: when schema expects boolean/number, send JSON bool/number (never quoted strings).",
                "Before every tool call/finalize, self-check: phase-allowed tool, required keys present, and JSON value types exactly match schema.",
                "Check state_summary.tool_memory_projection first and reuse cached discovery/schema context; avoid repeating identical discovery calls in one snapshot scope.",
                "For schema/topic and control-step contracts, use guide.get with canonical request shape; schema lookups are digest-first and should request {\"full\":true} only when digest is insufficient.",
                "guide.get examples: good {\"schema\":\"ais-plan-sketch/0.1.0\"}, {\"schema\":\"ais-plan-sketch/0.1.0\",\"full\":true}, {\"topic\":\"cel\"}; bad {\"schema\":{\"id\":\"ais-plan-sketch/0.1.0\"}}, {\"full\":\"true\"}.",
                "Capability narrowing order: prefer catalog.search first (compact ref-first cards: ref/kind/chains?/risk_level?), then get_candidate_detail for selected refs; use list_candidates only as broad inventory when needed.",
                "list_candidates policy template (filter-first): start with exact chain, add protocol when hinted, and broaden only when empty/insufficient in strict order: exact chain+protocol -> exact chain -> chain namespace wildcard.",
                "assert/branch/until/retry are PlanSketch control-step semantics, not catalog candidates.",
                "candidate_ref is required for query/action steps and optional for assert/branch control steps.",
                "Plan against state_summary.todo_state.current_todo only and produce exactly one deterministic segment for that todo.",
                "depends_on may only reference step ids in the current segment; never use cross-segment refs like seg_1/....",
                "For inputs.* refs, use InputStore-only bindable refs: source_of_truth=state_summary.input_store, projection=state_summary.input_registry.known_refs; never invent candidate/protocol/action refs outside discovered context.",
                "For unknown_input_ref repair, preserve slot semantics: token/address params map to address-like refs (for example *.address), and *.decimals refs cannot substitute token/address slots.",
                "If previous_error.autofill.mode=host_binding_adjudicate_round: first prefer host-provided ambiguous_bindings[]/query_candidate_pool/available_input_refs; you may call readonly discovery tools (list_candidates/catalog.search/get_candidate_detail/guide.get) to find query refs; output binding/query decisions before asking user input.",
                "If previous_error.autofill.mode=host_missing_input_round: prioritize resolver.selected_query_refs/query_candidate_pool for recovery; attempt query_decisions/binding_decisions before emitting missing_required_input questions.",
                "Never ask user input until both input-ref binding and query-based recovery are exhausted for current missing refs.",
                "When resolver or host recovery has viable query/binding candidates, continue recovery and do not emit missing_required_input yet.",
                "Emit missing_required_input only after recovery exhaustion, and include error.details.recovery_exhaustion with unresolved_refs[] + reasons[] + attempt_trace_id.",
                "For missing_required_input, unresolved_refs/questions must use canonical source refs (inputs.* / node outputs) and must never expose params.* paths to users.",
                "For transfer/swap writes, enforce same-segment gate chain query -> assert|branch -> action and refresh volatile write facts by query when needed.",
                "For errors, return status=invalid|unavailable with error.reason_code; for missing_required_input use canonical shape: error.details.questions[] + error.details.recovery_exhaustion{unresolved_refs[],reasons[],attempt_trace_id} (all non-empty). Repair order is strict: shape -> ref -> slot -> semantic.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_begin: vec![
                "Current phase: begin.",
                "Allowed tools: plan.begin only.",
                "Call plan.begin exactly once.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_grounding: vec![
                "Current phase: ground_intent.",
                "Allowed tools: list_candidates, catalog.search, catalog.resolve_missing_facts, get_candidate_detail, guide.get, and one final plan.ground_intent (must be last).",
                "list_candidates usage follows the base-rules filter-first policy template; do not invent alternate broaden order.",
                "Goal: derive deterministic initial inputs/facts before todo planning; prioritize high-confidence owner/recipient/amount/token/chain fields and avoid guessing.",
                "When grounding-required facts for known refs are missing, call catalog.resolve_missing_facts with missing_refs before asking user input.",
                "If status=proposed and ready_for_todos=false, include actionable questions or missing_refs (non-empty).",
                "If required grounding fields remain missing, return unavailable with reason_code=missing_required_input and canonical error.details.questions[] + error.details.recovery_exhaustion.",
                "Call plan.ground_intent exactly once and only as the last tool call.",
                "Do not call plan.begin, plan.propose_todos, plan.propose_segment, or plan.revise_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_todos: vec![
                "Current phase: propose_todos.",
                "Allowed tools: list_candidates, catalog.search, catalog.resolve_missing_facts, get_candidate_detail, guide.get, and one final plan.propose_todos (must be last).",
                "list_candidates usage follows the base-rules filter-first policy template; do not invent alternate broaden order.",
                "Output deterministic todos for the whole intent before segment planning.",
                "Each todo must include title; optional fields: required_facts/produced_facts/acceptance.",
                "Prefer 2-4 concise todos; avoid duplicates or overlapping objectives.",
                "When todo-required facts for known refs are missing, call catalog.resolve_missing_facts with missing_refs before asking user input.",
                "Call plan.propose_todos exactly once and only as the last tool call.",
                "Do not call plan.begin, plan.propose_segment, or plan.revise_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_propose: vec![
                "Current phase: propose_segment.",
                "Allowed tools: list_candidates, catalog.search, catalog.resolve_missing_facts, get_candidate_detail, guide.get, plan.check_segment, and one final plan.propose_segment (must be last).",
                "list_candidates usage follows the base-rules filter-first policy template; do not invent alternate broaden order.",
                "Host enforces 1 todo = 1 segment; plan only for current state_summary.todo_state.current_todo.",
                "Segment shape must stay flat: do not output legacy branch-tree fields (if_true/if_false/then/else/children); encode branch paths via flat steps + when.cel + depends_on.",
                "You must call plan.check_segment and only finalize when check result has ok=true.",
                "If token decimals or write-required facts are unknown, call catalog.resolve_missing_facts with missing refs, then add corresponding query steps before write when possible; do not patch token/address slots with *.decimals refs.",
                "If required facts are missing and resolver/host recovery still has query or binding candidates, continue recovery and do not emit missing_required_input yet.",
                "If required facts are still missing after recovery is exhausted, return unavailable with reason_code=missing_required_input and include canonical error.details.questions[] + error.details.recovery_exhaustion.",
                "Call plan.propose_segment exactly once and only as the last tool call.",
                "Never call plan.begin or plan.revise_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_revise: vec![
                "Current phase: revise_segment.",
                "Allowed tools: list_candidates, catalog.search, catalog.resolve_missing_facts, get_candidate_detail, guide.get, plan.check_segment, and one final plan.revise_segment (must be last).",
                "list_candidates usage follows the base-rules filter-first policy template; do not invent alternate broaden order.",
                "Keep repairing the same current todo from state_summary.todo_state.current_todo; do not switch to a different objective.",
                "Apply minimum edits to fix output shape and keep semantics stable; patch previous_error.last_failed_finalize when available instead of regenerating from scratch.",
                "Segment shape must stay flat: do not output legacy branch-tree fields (if_true/if_false/then/else/children); encode branch paths via flat steps + when.cel + depends_on.",
                "You must call plan.check_segment and only finalize when check result has ok=true.",
                "Repair order is strict: shape -> ref -> slot -> semantic; keep semantic edits minimal.",
                "If decimals/facts are missing, call catalog.resolve_missing_facts and prefer adding matched query steps before returning missing_required_input; do not patch token/address slots with *.decimals refs.",
                "If required facts are missing and resolver/host recovery still has query or binding candidates, continue recovery and do not emit missing_required_input yet.",
                "If required facts are still missing after recovery is exhausted, return unavailable with reason_code=missing_required_input and include canonical error.details.questions[] + error.details.recovery_exhaustion.",
                "Call plan.revise_segment exactly once and only as the last tool call.",
                "Never call plan.begin or plan.propose_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            contracts_summary: vec![
                "ValueRef forms: lit/ref/cel/object/array.",
                "Asset shape: object.address + object.chain_ref (compiler normalizes to chain_id).",
                "Use CEL for deterministic conditions and value computation; expressions must be side-effect free.",
                "Write-safety should be expressed via deterministic CEL conditions and explicit query/assert/branch guards.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

impl SegmentedPromptContextBuilder {
    fn with_overrides(mut self, overrides: SegmentedPromptOverrides) -> Self {
        if let Some(base_rules) = non_empty_rules(overrides.base_rules) {
            self.base_rules = base_rules;
        }
        if let Some(rules) = non_empty_rules(overrides.phase_rules_begin) {
            self.phase_rules_begin = rules;
        }
        if let Some(rules) = non_empty_rules(overrides.phase_rules_grounding) {
            self.phase_rules_grounding = rules;
        }
        if let Some(rules) = non_empty_rules(overrides.phase_rules_todos) {
            self.phase_rules_todos = rules;
        }
        if let Some(rules) = non_empty_rules(overrides.phase_rules_propose) {
            self.phase_rules_propose = rules;
        }
        if let Some(rules) = non_empty_rules(overrides.phase_rules_revise) {
            self.phase_rules_revise = rules;
        }
        if let Some(contracts_summary) = non_empty_rules(overrides.contracts_summary) {
            self.contracts_summary = contracts_summary;
        }
        self
    }

    fn render(
        &self,
        phase: PlannerRoundPhase,
        candidate_context: Option<&CandidateContext>,
    ) -> PromptRenderOutput {
        let phase_rules = self.phase_rules(phase);
        let base_rules: Cow<'_, [String]> = Cow::Borrowed(self.base_rules.as_slice());
        let contracts_summary: Cow<'_, [String]> = Cow::Borrowed(self.contracts_summary.as_slice());
        let workspace_summary = workspace_summary(candidate_context);
        let pack_summary =
            "Planning snapshot source: request.snapshot_hash (derived from pack/catalog/chain_scope/approval mode).";
        let modules = json!({
            "version": SEGMENTED_PROMPT_VERSION,
            "phase": phase_name(phase),
            "base_rules": base_rules,
            "phase_rules": phase_rules,
            "contracts_summary": contracts_summary,
            "pack_summary": pack_summary,
            "workspace_summary": workspace_summary,
        });
        let hash = stable_hash_hex(&modules, &StableJsonOptions::default())
            .unwrap_or_else(|_| "prompt-hash-unavailable".to_string());
        let prompt = format!(
            "You are an AIS segmented planner.\nPrompt-Version: {SEGMENTED_PROMPT_VERSION}\nPrompt-Hash: {hash}\n\nBase Rules:\n{}\n\nPhase Rules:\n{}\n\nContracts Summary:\n{}\n\nPack Summary:\n- {pack_summary}\n\nWorkspace Summary:\n{}",
            numbered_lines(base_rules.as_ref()),
            numbered_lines(phase_rules.as_slice()),
            numbered_lines(contracts_summary.as_ref()),
            workspace_summary_lines(&workspace_summary)
        );
        PromptRenderOutput {
            version: SEGMENTED_PROMPT_VERSION.to_string(),
            hash,
            prompt,
        }
    }

    fn phase_rules(&self, phase: PlannerRoundPhase) -> &Vec<String> {
        match phase {
            PlannerRoundPhase::Begin => &self.phase_rules_begin,
            PlannerRoundPhase::GroundIntent => &self.phase_rules_grounding,
            PlannerRoundPhase::ProposeTodos => &self.phase_rules_todos,
            PlannerRoundPhase::ProposeSegment => &self.phase_rules_propose,
            PlannerRoundPhase::ReviseSegment => &self.phase_rules_revise,
        }
    }
}

impl<P> LlmSegmentedIntentPlanner<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            candidate_context: None,
            planning_memory: PlanningMemory::default(),
            diagnostics_tracker: PlannerDiagnosticsTracker::default(),
            last_failed_finalize: None,
            begin_context: None,
            prompt_builder: SegmentedPromptContextBuilder::default(),
            prompt_overrides: SegmentedPromptOverrides::default(),
            max_tool_rounds: DEFAULT_SEGMENTED_MAX_TOOL_ROUNDS,
            verbose_llm: false,
            usage_tracker: PlannerLlmUsageTracker::default(),
            llm_transcript: None,
        }
    }

    pub fn with_context_limit_tokens(mut self, context_limit_tokens: Option<usize>) -> Self {
        let mapped = context_limit_tokens.and_then(|value| u64::try_from(value).ok());
        self.usage_tracker = self.usage_tracker.clone().with_context_limit_tokens(mapped);
        self
    }

    pub fn with_max_tool_rounds(mut self, max_tool_rounds: u8) -> Self {
        self.max_tool_rounds = max_tool_rounds.max(1);
        self
    }

    pub fn with_candidate_context(mut self, candidate_context: Option<CandidateContext>) -> Self {
        self.candidate_context = candidate_context;
        self
    }

    pub fn with_verbose_llm(mut self, verbose_llm: bool) -> Self {
        self.verbose_llm = verbose_llm;
        self
    }

    pub fn with_llm_transcript(mut self, path: Option<PathBuf>, append: bool) -> Self {
        self.llm_transcript = path.map(|path| LlmTranscriptSink {
            path,
            append,
            initialized: false,
        });
        self
    }

    pub fn with_prompt_overrides(mut self, overrides: SegmentedPromptOverrides) -> Self {
        self.prompt_builder = self
            .prompt_builder
            .clone()
            .with_overrides(overrides.clone());
        self.prompt_overrides = overrides;
        self
    }

    pub fn restore_planning_memory_from_checkpoint(&mut self, value: Option<&Value>) -> bool {
        let Some(value) = value else {
            return false;
        };
        let budget = self.planning_memory.current_budget();
        self.planning_memory.restore_from_checkpoint(value, budget)
    }

    pub fn planning_memory_checkpoint_value(&self) -> Option<Value> {
        self.planning_memory
            .checkpoint_value(self.planning_memory.current_budget())
    }

    pub fn llm_usage_value(&self) -> Value {
        let mut usage = self.usage_tracker.to_value();
        if let Some(usage_object) = usage.as_object_mut() {
            usage_object.insert(
                "diagnostics".to_string(),
                self.diagnostics_tracker.to_value(),
            );
        }
        usage
    }

    pub fn tool_lifecycle_value(&self) -> Value {
        let diagnostics = self.diagnostics_tracker.to_value();
        json!({
            "schema": "ais-agent-tool-lifecycle/0.0.1",
            "counters": diagnostics.pointer("/tool_exec").cloned().unwrap_or(Value::Null),
            "parallel": {
                "batches_total": diagnostics.pointer("/parallel_batches_total").cloned().unwrap_or(Value::Null),
                "calls_total": diagnostics.pointer("/parallel_calls_total").cloned().unwrap_or(Value::Null),
                "failures_total": diagnostics.pointer("/parallel_failures_total").cloned().unwrap_or(Value::Null),
                "partial_success_total": diagnostics.pointer("/parallel_partial_success_total").cloned().unwrap_or(Value::Null),
            }
        })
    }

    #[allow(dead_code)]
    pub fn tool_memory_projection_value(&self, max_tokens: usize) -> Option<Value> {
        self.planning_memory.tool_memory_projection(max_tokens)
    }

    pub fn tool_memory_projection_candidates_value(
        &self,
        max_tokens: usize,
    ) -> ToolMemoryProjectionCandidates {
        self.planning_memory
            .tool_memory_projection_candidates(max_tokens)
    }

    pub(crate) fn set_planning_memory_budget(&mut self, budget: PlanningMemoryBudget) {
        self.planning_memory.set_budget(budget);
    }

    pub(crate) fn observe_tool_memory_projection(
        &mut self,
        budget_tokens: usize,
        estimated_tokens: Option<u64>,
    ) {
        self.diagnostics_tracker
            .observe_tool_memory_projection(budget_tokens, estimated_tokens);
    }

    pub fn take_last_failed_finalize(&mut self) -> Option<Value> {
        self.last_failed_finalize.take()
    }

    fn append_llm_transcript_entry(
        &mut self,
        phase: PlannerRoundPhase,
        finalize_tool: &str,
        round: u8,
        request: &CompleteWithToolsRequest,
        response: &ais_llm::CompleteWithToolsResponse,
    ) -> Result<(), RunnerError> {
        let Some(sink) = self.llm_transcript.as_mut() else {
            return Ok(());
        };
        if !sink.initialized {
            if !sink.append {
                std::fs::write(&sink.path, "").map_err(|source| RunnerError::WriteFile {
                    path: sink.path.display().to_string(),
                    source,
                })?;
            }
            sink.initialized = true;
        }

        let request_text = serde_json::to_string_pretty(&llm_request_to_value(request))
            .map_err(RunnerError::from)?;
        let response_text = serde_json::to_string_pretty(&llm_response_to_value(response))
            .map_err(RunnerError::from)?;
        let entry = format!(
            "\n## LLM Round\n- phase: `{}`\n- finalize_tool: `{}`\n- round: `{}`\n\n### Request\n```json\n{}\n```\n\n### Response\n```json\n{}\n```\n",
            phase_name(phase),
            finalize_tool,
            round,
            request_text,
            response_text
        );
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&sink.path)
            .map_err(|source| RunnerError::WriteFile {
                path: sink.path.display().to_string(),
                source,
            })?;
        file.write_all(entry.as_bytes())
            .map_err(|source| RunnerError::WriteFile {
                path: sink.path.display().to_string(),
                source,
            })?;
        Ok(())
    }

    fn run_with_finalize_tool(
        &mut self,
        user_prompt: String,
        finalize_tool: &str,
        segment_check_context: Option<&SegmentCheckContext>,
    ) -> Result<PlannerToolOutput, RunnerError>
    where
        P: LlmProvider,
    {
        self.last_failed_finalize = None;
        let phase = phase_from_finalize_tool(finalize_tool)?;
        let rendered_prompt = self
            .prompt_builder
            .render(phase, self.candidate_context.as_ref());
        let system_prompt = rendered_prompt.prompt.clone();
        let require_successful_segment_check =
            requires_successful_check_before_finalize(phase, segment_check_context);
        let mut latest_segment_check_ok = !require_successful_segment_check;
        let mut latest_checked_segment_signature: Option<String> = None;
        let mut messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: Some(system_prompt.clone()),
                tool_name: None,
                tool_call_id: None,
                tool_calls: vec![],
            },
            LlmMessage {
                role: MessageRole::User,
                content: Some(user_prompt),
                tool_name: None,
                tool_call_id: None,
                tool_calls: vec![],
            },
        ];
        let round_signal = extract_round_context_signal(
            messages
                .get(1)
                .and_then(|message| message.content.as_deref())
                .unwrap_or_default(),
        );
        let effective_max_tool_rounds = if round_signal.adjudicate_mode {
            self.max_tool_rounds.min(ADJUDICATE_MAX_TOOL_ROUNDS)
        } else {
            self.max_tool_rounds
        };
        let mut loop_guard = CatalogSearchLoopGuard::default();
        let mut plan_check_failure_guard = PlanCheckFailureLoopGuard::default();
        let mut control_step_ref_hint_emitted = false;
        let mut finalize_schema_repair_attempts = 0u8;
        let mut check_segment_schema_repair_attempts = 0u8;
        let mut no_toolcall_retry_attempts = 0u8;
        let tools = segmented_planner_tools_for_phase(phase);
        if self.verbose_llm {
            eprintln!(
                "[llm] segmented planner system_prompt={}",
                truncate_for_log(system_prompt.as_str(), 600)
            );
            eprintln!(
                "[llm] segmented planner prompt_meta version={} phase={} hash={}",
                rendered_prompt.version,
                phase_name(phase),
                rendered_prompt.hash
            );
            if let Some(user_prompt) = messages.get(1).and_then(|message| message.content.as_ref())
            {
                eprintln!(
                    "[llm] segmented planner user_prompt={}",
                    truncate_for_log(user_prompt.as_str(), 900)
                );
            }
            for tool in &tools {
                eprintln!(
                    "[llm] segmented planner tool_def name={} input_schema={}",
                    tool.name,
                    truncate_for_log(tool.input_schema.to_string().as_str(), 600)
                );
            }
        }

        for round in 0..effective_max_tool_rounds {
            self.diagnostics_tracker.observe_phase_round(phase);
            let llm_request = CompleteWithToolsRequest {
                messages: messages.clone(),
                tools: tools.clone(),
            };
            let response = self
                .provider
                .complete_with_tools(llm_request.clone())
                .map_err(|error| RunnerError::Llm(error.to_string()))?;
            self.append_llm_transcript_entry(
                phase,
                finalize_tool,
                round.saturating_add(1),
                &llm_request,
                &response,
            )?;
            let usage = self.usage_tracker.record_estimated(&llm_request, &response);
            if self.verbose_llm {
                eprintln!(
                    "[llm] segmented planner round={} finalize_tool={} tool_calls={}",
                    round + 1,
                    finalize_tool,
                    response.tool_calls.len()
                );
                eprintln!(
                    "[llm] segmented planner usage round={} input_tokens={} output_tokens={} total_tokens={} estimated={} source={} cumulative_calls={} cumulative_input_tokens={} cumulative_output_tokens={} cumulative_total_tokens={} context_limit_tokens={} context_soft_limit_tokens={} context_remaining_tokens={}",
                    round + 1,
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.total_tokens,
                    usage.estimated,
                    usage.source,
                    self.usage_tracker.calls,
                    usage.cumulative_input_tokens,
                    usage.cumulative_output_tokens,
                    usage.cumulative_total_tokens,
                    usage
                        .context_limit_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    usage
                        .context_soft_limit_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    usage
                        .context_remaining_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                );
                for call in &response.tool_calls {
                    eprintln!(
                        "[llm] segmented planner tool_call_id={} tool={} args={}",
                        call.id,
                        call.name,
                        truncate_for_log(call.arguments.to_string().as_str(), 600)
                    );
                }
            }

            if response.tool_calls.is_empty() {
                let payload = no_toolcall_repair_payload(
                    phase,
                    finalize_tool,
                    round.saturating_add(1),
                    no_toolcall_retry_attempts.saturating_add(1),
                    NO_TOOLCALL_RETRY_ATTEMPT_LIMIT,
                    &tools,
                );
                if no_toolcall_retry_attempts < NO_TOOLCALL_RETRY_ATTEMPT_LIMIT {
                    no_toolcall_retry_attempts = no_toolcall_retry_attempts.saturating_add(1);
                    self.diagnostics_tracker.observe_no_toolcall_retry();
                    super::trace::emit(
                        self.verbose_llm,
                        phase_name(phase),
                        "no_toolcall_retry",
                        &[
                            ("tool", finalize_tool.to_string()),
                            ("retry", no_toolcall_retry_attempts.to_string()),
                            ("max_retries", NO_TOOLCALL_RETRY_ATTEMPT_LIMIT.to_string()),
                        ],
                    );
                    messages.push(LlmMessage {
                        role: MessageRole::Assistant,
                        content: response.assistant_content.clone(),
                        tool_name: None,
                        tool_call_id: None,
                        tool_calls: vec![],
                    });
                    let payload_text =
                        serde_json::to_string(&payload).map_err(RunnerError::from)?;
                    messages.push(LlmMessage {
                        role: MessageRole::User,
                        content: Some(payload_text),
                        tool_name: None,
                        tool_call_id: None,
                        tool_calls: vec![],
                    });
                    continue;
                }
                self.diagnostics_tracker
                    .observe_no_toolcall_retry_exhausted();
                super::trace::emit(
                    self.verbose_llm,
                    phase_name(phase),
                    "no_toolcall_retry_exhausted",
                    &[
                        ("tool", finalize_tool.to_string()),
                        ("max_retries", NO_TOOLCALL_RETRY_ATTEMPT_LIMIT.to_string()),
                    ],
                );
                let payload_text =
                    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
                return Err(RunnerError::Llm(format!(
                    "segmented planner provider returned no tool calls: no_tool_calls_retries_exhausted payload={payload_text}"
                )));
            }
            validate_tool_calls_for_phase(&response.tool_calls, phase)?;

            messages.push(LlmMessage {
                role: MessageRole::Assistant,
                content: response.assistant_content.clone(),
                tool_name: None,
                tool_call_id: None,
                tool_calls: response.tool_calls.clone(),
            });
            if self.verbose_llm {
                eprintln!(
                    "[llm] segmented planner assistant_content={}",
                    truncate_for_log(response.assistant_content.as_deref().unwrap_or(""), 600)
                );
            }

            let mut tool_results = Vec::<LlmMessage>::new();
            let mut round_memory_hits = 0u64;
            let mut round_loop_hint: Option<Value> = None;
            let mut round_control_ref_hint: Option<Value> = None;
            let all_parallel_readonly = !response.tool_calls.is_empty()
                && response.tool_calls.iter().all(|call| {
                    call.name != finalize_tool && is_parallel_readonly_tool(call.name.as_str())
                });
            if all_parallel_readonly {
                let planner_usage = self.llm_usage_value();
                let projection_budget_tokens =
                    ToolMemoryBudgetPolicy::derive_tool_memory_projection_token_budget(
                        Some(&planner_usage),
                        None,
                    );
                let remaining_tokens = planner_usage
                    .get("context_remaining_tokens")
                    .and_then(Value::as_u64);
                let usage_ratio_bps = planner_usage
                    .get("context_soft_limit_tokens")
                    .and_then(Value::as_u64)
                    .and_then(|soft_limit| {
                        if soft_limit == 0 {
                            None
                        } else {
                            Some(10_000_u64.saturating_sub(
                                remaining_tokens.unwrap_or(0).saturating_mul(10_000) / soft_limit,
                            ))
                        }
                    });
                let pressure_mode = ToolMemoryBudgetPolicy::derive_context_pressure_mode(
                    usage_ratio_bps,
                    remaining_tokens,
                );
                let compress_level =
                    ToolMemoryBudgetPolicy::derive_global_compress_level(pressure_mode);
                self.diagnostics_tracker
                    .observe_parallel_batch(response.tool_calls.len() as u64);

                let mut call_results =
                    Vec::<(ToolCall, Result<DecodedSegmentedToolCall, RunnerError>, u64)>::new();
                for call in &response.tool_calls {
                    let started_at = Instant::now();
                    super::trace::emit(
                        self.verbose_llm,
                        phase_name(phase),
                        "planner_tool_exec_start",
                        &[
                            ("tool_call_id", call.id.clone()),
                            ("tool", call.name.clone()),
                            ("execution_mode", "parallel".to_string()),
                        ],
                    );
                    let normalized_args =
                        normalize_tool_args_for_validation(call.name.as_str(), &call.arguments);
                    let effective_arguments = if normalized_args.changed() {
                        normalized_args.arguments
                    } else {
                        call.arguments.clone()
                    };
                    let dedupe_key = super::tools::cache::tool_cache_key(
                        call.name.as_str(),
                        &effective_arguments,
                    );
                    let _ = self
                        .diagnostics_tracker
                        .observe_tool_call(call.name.as_str(), dedupe_key);
                    let mut effective_call = call.clone();
                    effective_call.arguments = effective_arguments;
                    let cache_key = super::tools::cache::tool_cache_key(
                        effective_call.name.as_str(),
                        &effective_call.arguments,
                    );
                    if let Some(cache_key) = cache_key.as_deref() {
                        if let Some(content) = self.planning_memory.get(cache_key) {
                            call_results.push((
                                call.clone(),
                                Ok(DecodedSegmentedToolCall::ToolMessage {
                                    tool_name: call.name.clone(),
                                    tool_call_id: call.id.clone(),
                                    content: content.to_string(),
                                    cached: true,
                                }),
                                started_at.elapsed().as_millis() as u64,
                            ));
                            continue;
                        }
                    }
                    let decoded = decode_segmented_tool_call_with_memory(
                        &effective_call,
                        finalize_tool,
                        phase,
                        self.candidate_context.as_ref(),
                        segment_check_context,
                        None,
                        Some(projection_budget_tokens),
                        Some(compress_level),
                    );
                    let latency_ms = started_at.elapsed().as_millis() as u64;
                    if let (
                        Ok(DecodedSegmentedToolCall::ToolMessage {
                            content,
                            cached: false,
                            ..
                        }),
                        Some(cache_key),
                    ) = (&decoded, cache_key)
                    {
                        self.planning_memory.insert(cache_key, content.clone());
                    }
                    call_results.push((call.clone(), decoded, latency_ms));
                }

                let mut parallel_errors = 0u64;
                for (call, decoded, latency_ms) in call_results {
                    match decoded {
                        Ok(DecodedSegmentedToolCall::ToolMessage {
                            tool_name,
                            tool_call_id,
                            content,
                            cached,
                        }) => {
                            self.diagnostics_tracker
                                .observe_tool_result(tool_name.as_str(), cached);
                            if cached {
                                round_memory_hits = round_memory_hits.saturating_add(1);
                            }
                            self.diagnostics_tracker.observe_tool_exec_end(
                                tool_name.as_str(),
                                "parallel",
                                if cached { "cached_hit" } else { "success" },
                                latency_ms,
                            );
                            super::trace::emit(
                                self.verbose_llm,
                                phase_name(phase),
                                "planner_tool_exec_end",
                                &[
                                    ("tool_call_id", tool_call_id.clone()),
                                    ("tool", tool_name.clone()),
                                    (
                                        "status",
                                        if cached { "cached_hit" } else { "success" }.to_string(),
                                    ),
                                    ("execution_mode", "parallel".to_string()),
                                    ("latency_ms", latency_ms.to_string()),
                                ],
                            );
                            if tool_name == "catalog.search" {
                                let signature =
                                    catalog_search_signature_from_result(content.as_str());
                                let is_empty = catalog_search_result_is_empty(content.as_str());
                                if is_empty {
                                    let should_hint = loop_guard.observe_empty(signature);
                                    self.diagnostics_tracker
                                        .observe_empty_search_streak(loop_guard.max_streak());
                                    if round_signal.adjudicate_mode
                                        && loop_guard.max_streak()
                                            >= ADJUDICATE_EMPTY_SEARCH_STREAK_LIMIT
                                    {
                                        round_loop_hint = Some(adjudicate_finalize_guard_payload(
                                            finalize_tool,
                                            round.saturating_add(1),
                                            "empty_catalog_search_streak",
                                        ));
                                    } else if should_hint {
                                        round_loop_hint =
                                            Some(catalog_search_loop_guard_hint_payload(
                                                loop_guard.current_streak,
                                            ));
                                    }
                                } else {
                                    loop_guard.observe_non_empty();
                                }
                            }
                            tool_results.push(LlmMessage {
                                role: MessageRole::Tool,
                                content: Some(content),
                                tool_name: Some(tool_name),
                                tool_call_id: Some(tool_call_id),
                                tool_calls: vec![],
                            });
                        }
                        Ok(DecodedSegmentedToolCall::Final(_)) => {
                            parallel_errors = parallel_errors.saturating_add(1);
                            self.diagnostics_tracker.observe_tool_exec_end(
                                call.name.as_str(),
                                "parallel",
                                "error",
                                latency_ms,
                            );
                            let content = planner_tool_error_payload(
                                call.name.as_str(),
                                call.id.as_str(),
                                "parallel_tool_unexpected_finalize",
                                "readonly parallel batch produced finalize tool output unexpectedly",
                            );
                            tool_results.push(LlmMessage {
                                role: MessageRole::Tool,
                                content: Some(content),
                                tool_name: Some(call.name.clone()),
                                tool_call_id: Some(call.id.clone()),
                                tool_calls: vec![],
                            });
                        }
                        Err(error) => {
                            parallel_errors = parallel_errors.saturating_add(1);
                            self.diagnostics_tracker.observe_tool_exec_end(
                                call.name.as_str(),
                                "parallel",
                                "error",
                                latency_ms,
                            );
                            super::trace::emit(
                                self.verbose_llm,
                                phase_name(phase),
                                "planner_tool_exec_end",
                                &[
                                    ("tool_call_id", call.id.clone()),
                                    ("tool", call.name.clone()),
                                    ("status", "error".to_string()),
                                    ("execution_mode", "parallel".to_string()),
                                    ("latency_ms", latency_ms.to_string()),
                                    ("error", error.to_string()),
                                ],
                            );
                            let content = planner_tool_error_payload(
                                call.name.as_str(),
                                call.id.as_str(),
                                "planner_tool_execution_failed",
                                error.to_string().as_str(),
                            );
                            tool_results.push(LlmMessage {
                                role: MessageRole::Tool,
                                content: Some(content),
                                tool_name: Some(call.name.clone()),
                                tool_call_id: Some(call.id.clone()),
                                tool_calls: vec![],
                            });
                        }
                    }
                }
                if parallel_errors > 0 && (parallel_errors as usize) < response.tool_calls.len() {
                    self.diagnostics_tracker.observe_parallel_partial_success();
                }
                if tool_results.is_empty() {
                    return Err(RunnerError::Llm(
                        "segmented planner returned no actionable tools".to_string(),
                    ));
                }
                messages.extend(tool_results);
                if let Some(loop_hint) = round_loop_hint {
                    let hint_text = serde_json::to_string_pretty(&loop_hint)
                        .unwrap_or_else(|_| "{}".to_string());
                    messages.push(LlmMessage {
                        role: MessageRole::User,
                        content: Some(hint_text),
                        tool_name: None,
                        tool_call_id: None,
                        tool_calls: vec![],
                    });
                }
                if self.verbose_llm {
                    let duplicate_ratio_bps = self.diagnostics_tracker.duplicate_ratio_bps();
                    eprintln!(
                        "[llm] segmented planner round_summary round={} phase={} pressure_mode={} compressed={} memory_hits={} duplicate_ratio_bps={} empty_search_streak_max={} adjudicate_mode={} max_rounds={}",
                        round + 1,
                        phase_name(phase),
                        round_signal
                            .pressure_mode
                            .as_deref()
                            .unwrap_or("-"),
                        round_signal.compressed,
                        round_memory_hits,
                        duplicate_ratio_bps,
                        self.diagnostics_tracker.empty_search_streak_max,
                        round_signal.adjudicate_mode,
                        effective_max_tool_rounds,
                    );
                }
                continue;
            }
            for call in &response.tool_calls {
                let normalized_args =
                    normalize_tool_args_for_validation(call.name.as_str(), &call.arguments);
                if normalized_args.changed() {
                    super::trace::emit(
                        self.verbose_llm,
                        phase_name(phase),
                        "tool_args_normalized",
                        &[
                            ("tool_call_id", call.id.clone()),
                            ("tool", call.name.clone()),
                            ("fields", normalized_args.normalized_fields.join(",")),
                        ],
                    );
                }
                let effective_arguments = if normalized_args.changed() {
                    normalized_args.arguments
                } else {
                    call.arguments.clone()
                };
                let dedupe_key =
                    super::tools::cache::tool_cache_key(call.name.as_str(), &effective_arguments);
                let duplicate = self
                    .diagnostics_tracker
                    .observe_tool_call(call.name.as_str(), dedupe_key);
                if self.verbose_llm && duplicate {
                    eprintln!(
                        "[llm] segmented planner duplicate_tool_call round={} phase={} tool={} tool_call_id={}",
                        round + 1,
                        phase_name(phase),
                        call.name,
                        call.id
                    );
                }
                let tool_exec_started_at = Instant::now();
                super::trace::emit(
                    self.verbose_llm,
                    phase_name(phase),
                    "planner_tool_exec_start",
                    &[
                        ("tool_call_id", call.id.clone()),
                        ("tool", call.name.clone()),
                        ("execution_mode", "sequential".to_string()),
                    ],
                );
                let mut effective_call = call.clone();
                effective_call.arguments = effective_arguments;
                let planner_usage = self.llm_usage_value();
                let projection_budget_tokens =
                    ToolMemoryBudgetPolicy::derive_tool_memory_projection_token_budget(
                        Some(&planner_usage),
                        None,
                    );
                let remaining_tokens = planner_usage
                    .get("context_remaining_tokens")
                    .and_then(Value::as_u64);
                let usage_ratio_bps = planner_usage
                    .get("context_soft_limit_tokens")
                    .and_then(Value::as_u64)
                    .and_then(|soft_limit| {
                        if soft_limit == 0 {
                            None
                        } else {
                            Some(10_000_u64.saturating_sub(
                                remaining_tokens.unwrap_or(0).saturating_mul(10_000) / soft_limit,
                            ))
                        }
                    });
                let pressure_mode = ToolMemoryBudgetPolicy::derive_context_pressure_mode(
                    usage_ratio_bps,
                    remaining_tokens,
                );
                let compress_level =
                    ToolMemoryBudgetPolicy::derive_global_compress_level(pressure_mode);
                let decoded = match decode_segmented_tool_call_with_memory(
                    &effective_call,
                    finalize_tool,
                    phase,
                    self.candidate_context.as_ref(),
                    segment_check_context,
                    Some(&mut self.planning_memory),
                    Some(projection_budget_tokens),
                    Some(compress_level),
                ) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        if let Some(repair) = non_finalize_tool_args_repair_payload(
                            &error,
                            call.name.as_str(),
                            round.saturating_add(1),
                            check_segment_schema_repair_attempts.saturating_add(1),
                            NON_FINALIZE_TOOL_SCHEMA_REPAIR_ATTEMPT_LIMIT,
                        ) {
                            if call.name == "plan.check_segment"
                                && check_segment_schema_repair_attempts
                                    < NON_FINALIZE_TOOL_SCHEMA_REPAIR_ATTEMPT_LIMIT
                            {
                                self.diagnostics_tracker.observe_tool_exec_retry(false);
                                check_segment_schema_repair_attempts =
                                    check_segment_schema_repair_attempts.saturating_add(1);
                                super::trace::emit(
                                    self.verbose_llm,
                                    phase_name(phase),
                                    "tool_args_schema_repair_retry",
                                    &[
                                        ("tool", call.name.clone()),
                                        ("sub_reason_code", repair.sub_reason_code.to_string()),
                                        ("retry", check_segment_schema_repair_attempts.to_string()),
                                        (
                                            "max_retries",
                                            NON_FINALIZE_TOOL_SCHEMA_REPAIR_ATTEMPT_LIMIT
                                                .to_string(),
                                        ),
                                    ],
                                );
                                let content = serde_json::to_string(&repair.payload)
                                    .map_err(RunnerError::from)?;
                                if self.verbose_llm {
                                    eprintln!(
                                        "[llm] tool_result tool_call_id={} tool={} cached=false {}",
                                        call.id,
                                        call.name,
                                        summarize_tool_message(
                                            call.name.as_str(),
                                            content.as_str()
                                        )
                                    );
                                    eprintln!(
                                        "[llm] tool_result_prompt tool_call_id={} tool={} content={}",
                                        call.id,
                                        call.name,
                                        truncate_for_log(content.as_str(), 900)
                                    );
                                }
                                tool_results.push(LlmMessage {
                                    role: MessageRole::Tool,
                                    content: Some(content),
                                    tool_name: Some(call.name.clone()),
                                    tool_call_id: Some(call.id.clone()),
                                    tool_calls: vec![],
                                });
                                let latency_ms = tool_exec_started_at.elapsed().as_millis() as u64;
                                self.diagnostics_tracker.observe_tool_exec_end(
                                    call.name.as_str(),
                                    "sequential",
                                    "blocked_finalize",
                                    latency_ms,
                                );
                                continue;
                            }
                            super::trace::emit(
                                self.verbose_llm,
                                phase_name(phase),
                                "tool_args_schema_repair_exhausted",
                                &[
                                    ("tool", call.name.clone()),
                                    ("sub_reason_code", repair.sub_reason_code.to_string()),
                                    (
                                        "max_retries",
                                        NON_FINALIZE_TOOL_SCHEMA_REPAIR_ATTEMPT_LIMIT.to_string(),
                                    ),
                                ],
                            );
                            self.diagnostics_tracker.observe_tool_exec_retry(true);
                        }
                        if call.name == finalize_tool {
                            self.last_failed_finalize = Some(compact_failed_finalize_payload(
                                call,
                                response.assistant_content.as_deref(),
                                round.saturating_add(1),
                            ));
                            if let Some(repair) = finalize_schema_repair_payload(
                                &error,
                                finalize_tool,
                                round.saturating_add(1),
                                finalize_schema_repair_attempts.saturating_add(1),
                                FINALIZE_SCHEMA_REPAIR_ATTEMPT_LIMIT,
                            ) {
                                if finalize_schema_repair_attempts
                                    < FINALIZE_SCHEMA_REPAIR_ATTEMPT_LIMIT
                                {
                                    self.diagnostics_tracker.observe_tool_exec_retry(false);
                                    finalize_schema_repair_attempts =
                                        finalize_schema_repair_attempts.saturating_add(1);
                                    self.diagnostics_tracker
                                        .observe_finalize_schema_repair_attempt(
                                            repair.sub_reason_code,
                                        );
                                    super::trace::emit(
                                        self.verbose_llm,
                                        phase_name(phase),
                                        "finalize_schema_repair_retry",
                                        &[
                                            ("tool", finalize_tool.to_string()),
                                            ("sub_reason_code", repair.sub_reason_code.to_string()),
                                            ("retry", finalize_schema_repair_attempts.to_string()),
                                            (
                                                "max_retries",
                                                FINALIZE_SCHEMA_REPAIR_ATTEMPT_LIMIT.to_string(),
                                            ),
                                        ],
                                    );
                                    let content = serde_json::to_string(&repair.payload)
                                        .map_err(RunnerError::from)?;
                                    if self.verbose_llm {
                                        eprintln!(
                                            "[llm] tool_result tool_call_id={} tool={} cached=false {}",
                                            call.id,
                                            call.name,
                                            summarize_tool_message(
                                                call.name.as_str(),
                                                content.as_str()
                                            )
                                        );
                                        eprintln!(
                                            "[llm] tool_result_prompt tool_call_id={} tool={} content={}",
                                            call.id,
                                            call.name,
                                            truncate_for_log(content.as_str(), 900)
                                        );
                                    }
                                    tool_results.push(LlmMessage {
                                        role: MessageRole::Tool,
                                        content: Some(content),
                                        tool_name: Some(call.name.clone()),
                                        tool_call_id: Some(call.id.clone()),
                                        tool_calls: vec![],
                                    });
                                    continue;
                                }
                                self.diagnostics_tracker
                                    .observe_finalize_schema_repair_exhausted();
                                self.diagnostics_tracker.observe_tool_exec_retry(true);
                                super::trace::emit(
                                    self.verbose_llm,
                                    phase_name(phase),
                                    "finalize_schema_repair_exhausted",
                                    &[
                                        ("tool", finalize_tool.to_string()),
                                        ("sub_reason_code", repair.sub_reason_code.to_string()),
                                        (
                                            "max_retries",
                                            FINALIZE_SCHEMA_REPAIR_ATTEMPT_LIMIT.to_string(),
                                        ),
                                    ],
                                );
                            }
                        }
                        let latency_ms = tool_exec_started_at.elapsed().as_millis() as u64;
                        self.diagnostics_tracker.observe_tool_exec_end(
                            call.name.as_str(),
                            "sequential",
                            "error",
                            latency_ms,
                        );
                        super::trace::emit(
                            self.verbose_llm,
                            phase_name(phase),
                            "planner_tool_exec_end",
                            &[
                                ("tool_call_id", call.id.clone()),
                                ("tool", call.name.clone()),
                                ("status", "error".to_string()),
                                ("execution_mode", "sequential".to_string()),
                                ("latency_ms", latency_ms.to_string()),
                                ("error", error.to_string()),
                            ],
                        );
                        return Err(error);
                    }
                };
                match decoded {
                    DecodedSegmentedToolCall::Final(result) => {
                        if require_successful_segment_check
                            && finalized_segment_is_proposed(&result)
                        {
                            if !latest_segment_check_ok {
                                let payload = missing_pre_finalize_check_payload(finalize_tool);
                                let content =
                                    serde_json::to_string(&payload).map_err(RunnerError::from)?;
                                if self.verbose_llm {
                                    eprintln!(
                                        "[llm] tool_result tool_call_id={} tool={} cached=false {}",
                                        call.id,
                                        call.name,
                                        summarize_tool_message(
                                            call.name.as_str(),
                                            content.as_str()
                                        )
                                    );
                                    eprintln!(
                                        "[llm] tool_result_prompt tool_call_id={} tool={} content={}",
                                        call.id,
                                        call.name,
                                        truncate_for_log(content.as_str(), 900)
                                    );
                                }
                                tool_results.push(LlmMessage {
                                    role: MessageRole::Tool,
                                    content: Some(content),
                                    tool_name: Some(call.name.clone()),
                                    tool_call_id: Some(call.id.clone()),
                                    tool_calls: vec![],
                                });
                                continue;
                            }
                            let finalized_signature = finalized_segment_signature(&result);
                            if finalized_signature.is_none()
                                || finalized_signature != latest_checked_segment_signature
                            {
                                let payload = pre_finalize_segment_mismatch_payload(
                                    finalize_tool,
                                    latest_checked_segment_signature.as_deref(),
                                    finalized_signature.as_deref(),
                                );
                                let content =
                                    serde_json::to_string(&payload).map_err(RunnerError::from)?;
                                if self.verbose_llm {
                                    eprintln!(
                                        "[llm] tool_result tool_call_id={} tool={} cached=false {}",
                                        call.id,
                                        call.name,
                                        summarize_tool_message(
                                            call.name.as_str(),
                                            content.as_str()
                                        )
                                    );
                                    eprintln!(
                                        "[llm] tool_result_prompt tool_call_id={} tool={} content={}",
                                        call.id,
                                        call.name,
                                        truncate_for_log(content.as_str(), 900)
                                    );
                                }
                                tool_results.push(LlmMessage {
                                    role: MessageRole::Tool,
                                    content: Some(content),
                                    tool_name: Some(call.name.clone()),
                                    tool_call_id: Some(call.id.clone()),
                                    tool_calls: vec![],
                                });
                                let latency_ms = tool_exec_started_at.elapsed().as_millis() as u64;
                                self.diagnostics_tracker.observe_tool_exec_end(
                                    call.name.as_str(),
                                    "sequential",
                                    "blocked_finalize",
                                    latency_ms,
                                );
                                continue;
                            }
                        }
                        let latency_ms = tool_exec_started_at.elapsed().as_millis() as u64;
                        self.diagnostics_tracker.observe_tool_exec_end(
                            call.name.as_str(),
                            "sequential",
                            "success",
                            latency_ms,
                        );
                        super::trace::emit(
                            self.verbose_llm,
                            phase_name(phase),
                            "planner_tool_exec_end",
                            &[
                                ("tool_call_id", call.id.clone()),
                                ("tool", call.name.clone()),
                                ("status", "success".to_string()),
                                ("execution_mode", "sequential".to_string()),
                                ("latency_ms", latency_ms.to_string()),
                            ],
                        );
                        return Ok(result);
                    }
                    DecodedSegmentedToolCall::ToolMessage {
                        tool_name,
                        tool_call_id,
                        content,
                        cached,
                    } => {
                        self.diagnostics_tracker
                            .observe_tool_result(tool_name.as_str(), cached);
                        if cached {
                            round_memory_hits = round_memory_hits.saturating_add(1);
                        }
                        if tool_name == "plan.check_segment" {
                            latest_segment_check_ok = plan_check_result_ok(content.as_str());
                            latest_checked_segment_signature = if latest_segment_check_ok {
                                plan_check_segment_signature_from_tool_args(&call.arguments)
                            } else {
                                None
                            };
                            let repeated_failure_hit = plan_check_failure_guard
                                .observe(plan_check_failure_signature(content.as_str()));
                            if repeated_failure_hit {
                                let payload = repeated_plan_check_failure_payload(
                                    content.as_str(),
                                    plan_check_failure_guard.streak(),
                                    REPEATED_PLAN_CHECK_FAILURE_THRESHOLD,
                                    finalize_tool,
                                );
                                let payload_text = serde_json::to_string(&payload)
                                    .unwrap_or_else(|_| "{}".to_string());
                                if self.verbose_llm {
                                    eprintln!(
                                        "[llm] segmented planner repeated_check_failure round={} phase={} payload={}",
                                        round + 1,
                                        phase_name(phase),
                                        truncate_for_log(payload_text.as_str(), 900)
                                    );
                                }
                                return Err(RunnerError::Llm(format!(
                                    "segmented planner repeated plan.check_segment failure: {payload_text}"
                                )));
                            }
                            if !control_step_ref_hint_emitted
                                && plan_check_has_control_step_candidate_not_found(content.as_str())
                            {
                                control_step_ref_hint_emitted = true;
                                round_control_ref_hint =
                                    Some(control_step_candidate_ref_hint_payload());
                            }
                        }
                        if tool_name == "catalog.search" {
                            let signature = catalog_search_signature_from_result(content.as_str());
                            let is_empty = catalog_search_result_is_empty(content.as_str());
                            if is_empty {
                                let should_hint = loop_guard.observe_empty(signature);
                                self.diagnostics_tracker
                                    .observe_empty_search_streak(loop_guard.max_streak());
                                if round_signal.adjudicate_mode
                                    && loop_guard.max_streak()
                                        >= ADJUDICATE_EMPTY_SEARCH_STREAK_LIMIT
                                {
                                    round_loop_hint = Some(adjudicate_finalize_guard_payload(
                                        finalize_tool,
                                        round.saturating_add(1),
                                        "empty_catalog_search_streak",
                                    ));
                                } else if should_hint {
                                    round_loop_hint = Some(catalog_search_loop_guard_hint_payload(
                                        loop_guard.current_streak,
                                    ));
                                }
                            } else {
                                loop_guard.observe_non_empty();
                            }
                        }
                        if self.verbose_llm {
                            eprintln!(
                                "[llm] tool_result tool_call_id={} tool={} cached={} {}",
                                tool_call_id,
                                tool_name,
                                cached,
                                summarize_tool_message(tool_name.as_str(), content.as_str())
                            );
                            eprintln!(
                                "[llm] tool_result_prompt tool_call_id={} tool={} content={}",
                                tool_call_id,
                                tool_name,
                                truncate_for_log(content.as_str(), 900)
                            );
                        }
                        let latency_ms = tool_exec_started_at.elapsed().as_millis() as u64;
                        self.diagnostics_tracker.observe_tool_exec_end(
                            tool_name.as_str(),
                            "sequential",
                            if cached { "cached_hit" } else { "success" },
                            latency_ms,
                        );
                        super::trace::emit(
                            self.verbose_llm,
                            phase_name(phase),
                            "planner_tool_exec_end",
                            &[
                                ("tool_call_id", tool_call_id.clone()),
                                ("tool", tool_name.clone()),
                                (
                                    "status",
                                    if cached { "cached_hit" } else { "success" }.to_string(),
                                ),
                                ("execution_mode", "sequential".to_string()),
                                ("latency_ms", latency_ms.to_string()),
                            ],
                        );
                        tool_results.push(LlmMessage {
                            role: MessageRole::Tool,
                            content: Some(content),
                            tool_name: Some(tool_name),
                            tool_call_id: Some(tool_call_id),
                            tool_calls: vec![],
                        });
                    }
                }
            }

            if tool_results.is_empty() {
                return Err(RunnerError::Llm(
                    "segmented planner returned no actionable tools".to_string(),
                ));
            }
            messages.extend(tool_results);
            if let Some(control_hint) = round_control_ref_hint {
                let hint_text = serde_json::to_string_pretty(&control_hint)
                    .unwrap_or_else(|_| "{}".to_string());
                if self.verbose_llm {
                    eprintln!(
                        "[llm] segmented planner control_ref_hint round={} phase={} hint={}",
                        round + 1,
                        phase_name(phase),
                        truncate_for_log(hint_text.as_str(), 600)
                    );
                }
                messages.push(LlmMessage {
                    role: MessageRole::User,
                    content: Some(hint_text),
                    tool_name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                });
            }
            if let Some(loop_hint) = round_loop_hint {
                let hint_text =
                    serde_json::to_string_pretty(&loop_hint).unwrap_or_else(|_| "{}".to_string());
                if self.verbose_llm {
                    eprintln!(
                        "[llm] segmented planner loop_guard round={} phase={} hint={}",
                        round + 1,
                        phase_name(phase),
                        truncate_for_log(hint_text.as_str(), 600)
                    );
                }
                messages.push(LlmMessage {
                    role: MessageRole::User,
                    content: Some(hint_text),
                    tool_name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                });
            }
            if self.verbose_llm {
                let duplicate_ratio_bps = self.diagnostics_tracker.duplicate_ratio_bps();
                eprintln!(
                    "[llm] segmented planner round_summary round={} phase={} pressure_mode={} compressed={} memory_hits={} duplicate_ratio_bps={} empty_search_streak_max={} adjudicate_mode={} max_rounds={}",
                    round + 1,
                    phase_name(phase),
                    round_signal
                        .pressure_mode
                        .as_deref()
                        .unwrap_or("-"),
                    round_signal.compressed,
                    round_memory_hits,
                    duplicate_ratio_bps,
                    self.diagnostics_tracker.empty_search_streak_max,
                    round_signal.adjudicate_mode,
                    effective_max_tool_rounds,
                );
            }
        }

        if round_signal.adjudicate_mode {
            let payload = adjudicate_finalize_guard_payload(
                finalize_tool,
                effective_max_tool_rounds,
                "adjudicate_round_limit_reached",
            );
            let payload_text =
                serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
            return Err(RunnerError::Llm(format!(
                "segmented planner adjudicate rounds exhausted: finalize_required payload={payload_text}"
            )));
        }
        Err(RunnerError::Llm(
            "segmented planner tool round limit reached".to_string(),
        ))
    }
}

impl<P> SegmentedIntentPlanner for LlmSegmentedIntentPlanner<P>
where
    P: LlmProvider,
{
    fn begin_session(
        &mut self,
        request: SegmentBeginRequest,
    ) -> Result<SegmentPlanningSession, RunnerError> {
        self.planning_memory.clear();
        self.diagnostics_tracker = PlannerDiagnosticsTracker::default();
        self.usage_tracker = PlannerLlmUsageTracker::default()
            .with_context_limit_tokens(self.usage_tracker.context_limit_tokens);
        self.begin_context = None;
        let output = self.run_with_finalize_tool(
            render_begin_prompt_with_patch(
                &request,
                self.prompt_overrides.begin_payload_patch.as_ref(),
            ),
            "plan.begin",
            None,
        )?;
        match output {
            PlannerToolOutput::Begin(session) => {
                self.planning_memory
                    .ensure_scope(session.session_id.as_str(), session.snapshot_hash.as_str());
                self.begin_context = Some(PlannerBeginContext {
                    pack_snapshot_hash: request.pack_snapshot_hash,
                    chain_scope: request.chain_scope,
                });
                Ok(session)
            }
            _ => Err(RunnerError::Llm(
                "segmented planner returned non-begin tool output for begin_session".to_string(),
            )),
        }
    }

    fn propose_segment(
        &mut self,
        request: SegmentPlanningRequest,
    ) -> Result<SegmentDraft, RunnerError> {
        self.planning_memory.ensure_scope(
            request.session.session_id.as_str(),
            request.session.snapshot_hash.as_str(),
        );
        let segment_check_context = self.segment_check_context(&request);
        let output = self.run_with_finalize_tool(
            render_segment_prompt_with_patch(
                "plan.propose_segment",
                &request,
                self.prompt_overrides.segment_payload_patch.as_ref(),
            ),
            "plan.propose_segment",
            segment_check_context.as_ref(),
        )?;
        match output {
            PlannerToolOutput::SegmentDraft(draft) => Ok(draft),
            _ => Err(RunnerError::Llm(
                "segmented planner returned non-segment output for propose_segment".to_string(),
            )),
        }
    }

    fn propose_todos(&mut self, request: TodoPlanningRequest) -> Result<TodoDraft, RunnerError> {
        self.planning_memory.ensure_scope(
            request.session.session_id.as_str(),
            request.session.snapshot_hash.as_str(),
        );
        let output = self.run_with_finalize_tool(
            render_todos_prompt_with_patch(
                &request,
                self.prompt_overrides.todos_payload_patch.as_ref(),
            ),
            "plan.propose_todos",
            None,
        )?;
        match output {
            PlannerToolOutput::TodoDraft(draft) => Ok(draft),
            _ => Err(RunnerError::Llm(
                "segmented planner returned non-todo output for propose_todos".to_string(),
            )),
        }
    }

    fn ground_intent(
        &mut self,
        request: IntentGroundingRequest,
    ) -> Result<IntentGroundingDraft, RunnerError> {
        self.planning_memory.ensure_scope(
            request.session.session_id.as_str(),
            request.session.snapshot_hash.as_str(),
        );
        let output = self.run_with_finalize_tool(
            render_grounding_prompt_with_patch(
                &request,
                self.prompt_overrides.grounding_payload_patch.as_ref(),
            ),
            "plan.ground_intent",
            None,
        )?;
        match output {
            PlannerToolOutput::IntentGrounding(draft) => Ok(draft),
            _ => Err(RunnerError::Llm(
                "segmented planner returned non-grounding output for ground_intent".to_string(),
            )),
        }
    }

    fn revise_segment(
        &mut self,
        request: SegmentPlanningRequest,
    ) -> Result<SegmentDraft, RunnerError> {
        self.planning_memory.ensure_scope(
            request.session.session_id.as_str(),
            request.session.snapshot_hash.as_str(),
        );
        let segment_check_context = self.segment_check_context(&request);
        let output = self.run_with_finalize_tool(
            render_segment_prompt_with_patch(
                "plan.revise_segment",
                &request,
                self.prompt_overrides.segment_payload_patch.as_ref(),
            ),
            "plan.revise_segment",
            segment_check_context.as_ref(),
        )?;
        match output {
            PlannerToolOutput::SegmentDraft(draft) => Ok(draft),
            _ => Err(RunnerError::Llm(
                "segmented planner returned non-segment output for revise_segment".to_string(),
            )),
        }
    }
}

impl<P> LlmSegmentedIntentPlanner<P> {
    fn segment_check_context(
        &self,
        request: &SegmentPlanningRequest,
    ) -> Option<SegmentCheckContext> {
        let begin = self.begin_context.as_ref()?;
        Some(SegmentCheckContext {
            intent: request.intent.clone(),
            session_id: request.session.session_id.clone(),
            cursor: request.session.cursor.clone(),
            pack_snapshot_hash: begin.pack_snapshot_hash.clone(),
            chain_scope: begin.chain_scope.clone(),
            known_input_refs: super::known_input_refs_from_state_summary(
                request.state_summary.as_ref(),
            ),
            grounding_fact_keys: super::grounding_fact_keys_from_state_summary(
                request.state_summary.as_ref(),
            ),
            current_todo: request
                .state_summary
                .as_ref()
                .and_then(|summary| summary.pointer("/todo_state/current_todo"))
                .cloned(),
        })
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CandidateDetailArgs {
    pub(super) refs: Vec<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub(super) struct ListCandidatesFilterArgs {
    #[serde(default)]
    pub(super) chain: Option<String>,
    #[serde(default)]
    pub(super) protocol: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ListCandidatesArgs {
    #[serde(default)]
    pub(super) chain: Option<String>,
    #[serde(default)]
    pub(super) protocol: Option<String>,
    #[serde(default)]
    pub(super) filter: Option<ListCandidatesFilterArgs>,
}

impl ListCandidatesArgs {
    pub(super) fn normalized_filter(&self) -> ListCandidatesFilterArgs {
        let chain = self
            .filter
            .as_ref()
            .and_then(|filter| filter.chain.as_deref())
            .or(self.chain.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let protocol = self
            .filter
            .as_ref()
            .and_then(|filter| filter.protocol.as_deref())
            .or(self.protocol.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        ListCandidatesFilterArgs { chain, protocol }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct CatalogSearchArgs {
    #[serde(default)]
    pub(super) query: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) chain: Option<String>,
    #[serde(default)]
    pub(super) min_risk_level: Option<u8>,
    #[serde(default)]
    pub(super) max_risk_level: Option<u8>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct GuideGetArgs {
    #[serde(default)]
    pub(super) schema: Option<String>,
    #[serde(default)]
    pub(super) topic: Option<String>,
    #[serde(default)]
    pub(super) full: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ResolveMissingFactsArgs {
    #[serde(default)]
    pub(super) missing_refs: Vec<String>,
    #[serde(default)]
    pub(super) limit_per_ref: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CheckSegmentArgs {
    pub(super) segment: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct BeginLimits {
    pub(super) max_rounds: u8,
    pub(super) max_segments: u8,
}

#[derive(Debug, Deserialize)]
pub(super) struct BeginToolArgs {
    pub(super) session_id: Value,
    pub(super) snapshot_hash: Value,
    pub(super) cursor: Value,
    pub(super) limits: BeginLimits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
struct PlannerIssueObject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl PlannerIssueObject {
    fn into_value(self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum PlannerIssue {
    Typed(PlannerIssueObject),
    Raw(Value),
}

impl PlannerIssue {
    fn into_value(self) -> Value {
        match self {
            Self::Typed(typed) => typed.into_value(),
            Self::Raw(raw) => raw,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct PlannerIssueList(Vec<PlannerIssue>);

impl PlannerIssueList {
    fn into_values(self) -> Vec<Value> {
        self.0
            .into_iter()
            .map(PlannerIssue::into_value)
            .collect::<Vec<_>>()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum PlannerQuestion {
    Typed(MissingInputQuestion),
    Raw(Value),
}

impl PlannerQuestion {
    fn into_value(self) -> Value {
        match self {
            Self::Typed(typed) => serde_json::to_value(typed)
                .unwrap_or_else(|_| Value::Object(serde_json::Map::new())),
            Self::Raw(raw) => raw,
        }
    }

    fn into_missing_input_question(self) -> Option<MissingInputQuestion> {
        match self {
            Self::Typed(typed) => Some(typed),
            Self::Raw(raw) => serde_json::from_value::<MissingInputQuestion>(raw).ok(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct PlannerQuestionList(Vec<PlannerQuestion>);

impl PlannerQuestionList {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn to_values(&self) -> Vec<Value> {
        self.0
            .iter()
            .cloned()
            .map(PlannerQuestion::into_value)
            .collect::<Vec<_>>()
    }

    fn into_values(self) -> Vec<Value> {
        self.0
            .into_iter()
            .map(PlannerQuestion::into_value)
            .collect::<Vec<_>>()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlannerErrorDetailsObject {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    questions: Vec<PlannerQuestion>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl PlannerErrorDetailsObject {
    #[cfg(test)]
    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    }

    fn missing_input_questions(&self) -> Vec<MissingInputQuestion> {
        self.questions
            .iter()
            .cloned()
            .filter_map(PlannerQuestion::into_missing_input_question)
            .collect::<Vec<_>>()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum PlannerErrorDetails {
    Typed(PlannerErrorDetailsObject),
    Raw(Value),
}

impl PlannerErrorDetails {
    fn recovery_exhaustion_value(&self) -> Option<&Value> {
        match self {
            Self::Typed(typed) => typed.extra.get("recovery_exhaustion"),
            Self::Raw(raw) => raw.get("recovery_exhaustion"),
        }
    }

    #[cfg(test)]
    fn to_value(&self) -> Value {
        match self {
            Self::Typed(typed) => typed.to_value(),
            Self::Raw(raw) => raw.clone(),
        }
    }

    fn missing_input_questions(&self) -> Vec<MissingInputQuestion> {
        match self {
            Self::Typed(typed) => typed.missing_input_questions(),
            Self::Raw(raw) => raw
                .get("questions")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            serde_json::from_value::<MissingInputQuestion>(item.clone()).ok()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        }
    }

    fn recovery_exhaustion_unresolved_refs(&self) -> Vec<String> {
        let unresolved = self
            .recovery_exhaustion_value()
            .and_then(Value::as_object)
            .and_then(|object| object.get("unresolved_refs"));
        unresolved
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
    fn recovery_exhaustion_reasons(&self) -> Vec<String> {
        self.recovery_exhaustion_value()
            .and_then(Value::as_object)
            .and_then(|object| object.get("reasons"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn recovery_exhaustion_attempt_trace_id(&self) -> Option<String> {
        self.recovery_exhaustion_value()
            .and_then(Value::as_object)
            .and_then(|object| object.get("attempt_trace_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

#[derive(Debug, Deserialize)]
struct PlannerToolError {
    reason_code: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    details: Option<PlannerErrorDetails>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SegmentToolArgs {
    status: String,
    done: bool,
    #[serde(default)]
    segment: Option<Value>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    cursor_next: Option<Value>,
    #[serde(default)]
    issues: PlannerIssueList,
    #[serde(default)]
    error: Option<PlannerToolError>,
    #[serde(default)]
    questions: PlannerQuestionList,
}

#[derive(Debug, Deserialize)]
pub(super) struct TodoToolArgs {
    status: String,
    #[serde(default)]
    todos: Vec<TodoSpec>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    issues: PlannerIssueList,
    #[serde(default)]
    error: Option<PlannerToolError>,
    #[serde(default)]
    questions: PlannerQuestionList,
}

#[derive(Debug, Deserialize)]
pub(super) struct GroundingToolArgs {
    status: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    ready_for_todos: Option<bool>,
    #[serde(default)]
    resolved_inputs: BTreeMap<String, Value>,
    #[serde(default)]
    intent_facts: BTreeMap<String, Value>,
    #[serde(default)]
    confidence: BTreeMap<String, u8>,
    #[serde(default)]
    issues: PlannerIssueList,
    #[serde(default)]
    error: Option<PlannerToolError>,
    #[serde(default)]
    questions: PlannerQuestionList,
    #[serde(default)]
    missing_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MissingInputOption {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MissingInputQuestion {
    id: String,
    question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    options: Vec<MissingInputOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    required: Option<bool>,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
}

fn schema_guide_payload(topic: &str) -> Option<Value> {
    match topic {
        "cel" => Some(json!({
            "topic": "cel",
            "summary": "Use CEL for deterministic conditional gating and lightweight value computation. Keep expressions side-effect free.",
            "allowed_namespaces": ["inputs", "params", "nodes", "query", "calculated", "policy", "ctx", "contracts"],
            "node_ref_rule": "Use nodes.<step_id>.outputs.<field> and only reference step ids in the same segment. Do not use segment/step form.",
            "patterns": [
                {
                    "name": "gate_by_query_output",
                    "example": "nodes.q_balance.outputs.balance >= 1000000000000000000"
                },
                {
                    "name": "gate_by_input",
                    "example": "inputs.amount_atomic <= params.max_amount_atomic"
                },
                {
                    "name": "short_circuit_on_condition",
                    "example": "nodes.q_allowance.outputs.amount < inputs.amount_atomic"
                },
                {
                    "name": "compute_value_for_input",
                    "example": "params.amount_atomic * 99 / 100"
                }
            ],
            "avoid": [
                "Do not call external services in CEL.",
                "Do not reference unknown or cross-segment node ids."
            ]
        })),
        "valueref" => Some(json!({
            "topic": "valueref",
            "allowed_kinds": ["lit", "ref", "cel", "object", "array"],
            "examples": [
                {"amount": {"lit": "10"}},
                {"owner": {"ref": "inputs.owner"}},
                {"ok": {"cel": "nodes.q_balance.outputs.balance > 100"}}
            ],
            "asset_hint": {
                "preferred_shape": {"object":{"address":{"lit":"0x..."}, "chain_ref":{"lit":"eip155:1"}}},
                "normalized_at_compile": "chain_ref -> chain_id"
            }
        })),
        _ => None,
    }
}

pub(super) fn guide_get_payload(args: GuideGetArgs) -> Value {
    let schema_id = args.schema.as_ref().map(|value| value.trim().to_string());
    let schema_id = schema_id.filter(|value| !value.is_empty());
    let topic = args
        .topic
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let full_schema = args.full.unwrap_or(false);

    match (schema_id, topic) {
        (Some(_), Some(_)) => json!({
            "error": {
                "code": "invalid_request_shape",
                "message": "guide.get accepts exactly one request kind: {\"schema\":\"...\"} or {\"topic\":\"...\"}"
            }
        }),
        (None, None) => json!({
            "error": {
                "code": "missing_argument",
                "message": "guide.get requires {\"schema\":\"...\"} or {\"topic\":\"...\"}"
            }
        }),
        (Some(schema_id), None) => {
            if let Some(embedded) = get_json_schema(schema_id.as_str()) {
                let parsed = serde_json::from_str::<Value>(embedded.json).unwrap_or_else(|_| {
                    json!({
                        "$id": embedded.id,
                        "raw": embedded.json
                    })
                });
                let digest = schema_digest_payload(embedded.id, &parsed);
                let schema_payload = if full_schema {
                    json!({
                        "id": embedded.id,
                        "mode": "full",
                        "digest": digest,
                        "json": parsed
                    })
                } else {
                    json!({
                        "id": embedded.id,
                        "mode": "digest",
                        "digest": digest
                    })
                };
                json!({
                    "kind": "schema",
                    "schema": schema_payload
                })
            } else {
                json!({
                    "kind": "schema",
                    "schema": {
                        "id": schema_id
                    },
                    "error": {
                        "code": "schema_not_found"
                    }
                })
            }
        }
        (None, Some(topic)) => {
            if let Some(guide) = schema_guide_payload(topic.as_str()) {
                json!({
                    "kind": "topic",
                    "topic": guide
                })
            } else {
                json!({
                    "kind": "topic",
                    "topic": {
                        "requested": topic
                    },
                    "error": {
                        "code": "topic_not_found",
                        "supported": ["cel", "valueref"]
                    }
                })
            }
        }
    }
}

fn schema_digest_payload(schema_id: &str, schema: &Value) -> Value {
    let root_required = string_array_from_value(schema.get("required"), 24);
    let root_properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|object| sorted_object_keys(object, 32))
        .unwrap_or_default();
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .map(|object| sorted_object_keys(object, 32))
        .unwrap_or_default();

    let mut digest = serde_json::Map::<String, Value>::new();
    digest.insert(
        "schema_id".to_string(),
        Value::String(schema_id.to_string()),
    );
    if !root_required.is_empty() {
        digest.insert(
            "root_required".to_string(),
            Value::Array(root_required.into_iter().map(Value::String).collect()),
        );
    }
    if !root_properties.is_empty() {
        digest.insert(
            "root_properties".to_string(),
            Value::Array(root_properties.into_iter().map(Value::String).collect()),
        );
    }
    if !defs.is_empty() {
        digest.insert(
            "defs".to_string(),
            Value::Array(defs.into_iter().map(Value::String).collect()),
        );
    }

    let mut plan_sketch = serde_json::Map::<String, Value>::new();
    let segment_required = string_array_from_pointer(schema, "/$defs/segment/required", 16);
    if !segment_required.is_empty() {
        plan_sketch.insert(
            "segment_required".to_string(),
            Value::Array(segment_required.into_iter().map(Value::String).collect()),
        );
    }
    let step_required = string_array_from_pointer(schema, "/$defs/step/required", 16);
    if !step_required.is_empty() {
        plan_sketch.insert(
            "step_required".to_string(),
            Value::Array(step_required.into_iter().map(Value::String).collect()),
        );
    }
    let step_kind_enum = string_array_from_pointer(schema, "/$defs/step/properties/kind/enum", 16);
    if !step_kind_enum.is_empty() {
        plan_sketch.insert(
            "step_kind_enum".to_string(),
            Value::Array(step_kind_enum.into_iter().map(Value::String).collect()),
        );
    }
    let retry_backoff =
        string_array_from_pointer(schema, "/$defs/retry_policy/properties/backoff/enum", 8);
    if !retry_backoff.is_empty() {
        plan_sketch.insert(
            "retry_backoff".to_string(),
            Value::Array(retry_backoff.into_iter().map(Value::String).collect()),
        );
    }
    if !plan_sketch.is_empty() {
        digest.insert("plan_sketch".to_string(), Value::Object(plan_sketch));
    }

    Value::Object(digest)
}

fn string_array_from_pointer(schema: &Value, pointer: &str, limit: usize) -> Vec<String> {
    string_array_from_value(schema.pointer(pointer), limit)
}

fn string_array_from_value(value: Option<&Value>, limit: usize) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .take(limit.max(1))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn sorted_object_keys(object: &serde_json::Map<String, Value>, limit: usize) -> Vec<String> {
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys.truncate(limit.max(1));
    keys
}

#[cfg(test)]
fn decode_segmented_tool_call(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    super::tools::dispatch::decode_segmented_tool_call(
        tool,
        finalize_tool,
        phase,
        candidate_context,
    )
}

fn decode_segmented_tool_call_with_memory(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
    segment_check_context: Option<&SegmentCheckContext>,
    memory: Option<&mut PlanningMemory>,
    projection_budget_tokens: Option<usize>,
    compress_level: Option<super::context::packing::ContextCompressLevel>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    super::tools::dispatch::decode_segmented_tool_call_with_memory(
        tool,
        finalize_tool,
        phase,
        candidate_context,
        segment_check_context,
        memory,
        projection_budget_tokens,
        compress_level,
    )
}

fn is_parallel_readonly_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "catalog.search" | "get_candidate_detail" | "guide.get" | "list_candidates"
    )
}

fn planner_tool_error_payload(
    tool_name: &str,
    tool_call_id: &str,
    reason_code: &str,
    message: &str,
) -> String {
    serde_json::to_string(&json!({
        "ok": false,
        "reason_code": reason_code,
        "tool": tool_name,
        "tool_call_id": tool_call_id,
        "message": message,
        "retryable": true,
        "details": {},
    }))
    .unwrap_or_else(|_| "{\"ok\":false}".to_string())
}

#[derive(Debug, Clone, Serialize)]
struct MissingFactQueryCandidate {
    query_ref: String,
    score: u16,
    matched_return_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct MissingFactResolution {
    missing_ref: String,
    query_candidates: Vec<MissingFactQueryCandidate>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ResolveMissingFactsPayload {
    schema: String,
    requested_missing_refs: usize,
    normalized_missing_refs: Vec<String>,
    limit_per_ref: usize,
    resolved: Vec<MissingFactResolution>,
    unresolved_refs: Vec<String>,
}

pub(super) fn resolve_missing_facts_payload(
    context: &CandidateContext,
    args: &ResolveMissingFactsArgs,
) -> Value {
    let mut normalized_missing_refs = args
        .missing_refs
        .iter()
        .filter_map(|raw| normalize_missing_fact_ref(raw.as_str()))
        .collect::<Vec<_>>();
    normalized_missing_refs.sort();
    normalized_missing_refs.dedup();
    let limit_per_ref = args.limit_per_ref.unwrap_or(3).clamp(1, 8);

    let mut resolved = Vec::<MissingFactResolution>::new();
    let mut unresolved_refs = Vec::<String>::new();
    for missing_ref in &normalized_missing_refs {
        let (query_candidates, truncated) =
            resolve_query_candidates_for_missing_ref(context, missing_ref.as_str(), limit_per_ref);
        if query_candidates.is_empty() {
            unresolved_refs.push(missing_ref.clone());
            continue;
        }
        resolved.push(MissingFactResolution {
            missing_ref: missing_ref.clone(),
            query_candidates,
            truncated,
        });
    }

    serde_json::to_value(ResolveMissingFactsPayload {
        schema: "ais-catalog-missing-fact-resolution/0.0.1".to_string(),
        requested_missing_refs: args.missing_refs.len(),
        normalized_missing_refs,
        limit_per_ref,
        resolved,
        unresolved_refs,
    })
    .unwrap_or_else(|_| json!({ "schema": "ais-catalog-missing-fact-resolution/0.0.1" }))
}

pub(crate) fn resolve_missing_facts_for_refs(
    candidate_context: &CandidateContext,
    missing_refs: &[String],
    limit_per_ref: usize,
) -> Value {
    let args = ResolveMissingFactsArgs {
        missing_refs: missing_refs.to_vec(),
        limit_per_ref: Some(limit_per_ref),
    };
    resolve_missing_facts_payload(candidate_context, &args)
}

fn resolve_query_candidates_for_missing_ref(
    context: &CandidateContext,
    missing_ref: &str,
    limit: usize,
) -> (Vec<MissingFactQueryCandidate>, bool) {
    let missing_key = missing_ref.strip_prefix("inputs.").unwrap_or(missing_ref);
    let missing_tokens = normalized_tokens(missing_key);
    let Some(leaf_token) = missing_tokens.last().cloned() else {
        return (Vec::new(), false);
    };

    let mut matches = Vec::<MissingFactQueryCandidate>::new();
    for query_card in &context.executable_candidates.queries {
        let Some(query_ref) = query_card.get("ref").and_then(Value::as_str) else {
            continue;
        };
        let query_tokens = normalized_tokens(query_ref);
        let return_names = context
            .detail_by_ref
            .get(query_ref)
            .map(query_return_field_names)
            .unwrap_or_default();

        let mut matched_return_fields = Vec::<String>::new();
        let mut score = 0u16;

        if query_tokens.contains(&leaf_token) {
            score = score.saturating_add(20);
        }
        for token in missing_tokens
            .iter()
            .take(missing_tokens.len().saturating_sub(1))
        {
            if query_tokens.contains(token) {
                score = score.saturating_add(4);
            }
        }

        for return_name in &return_names {
            let normalized_name = normalized_identifier(return_name);
            if normalized_name.is_empty() {
                continue;
            }
            if normalized_name == leaf_token {
                score = score.saturating_add(100);
                matched_return_fields.push(return_name.clone());
            } else if normalized_name.contains(leaf_token.as_str())
                || leaf_token.contains(normalized_name.as_str())
            {
                score = score.saturating_add(40);
                matched_return_fields.push(return_name.clone());
            }
            for token in missing_tokens
                .iter()
                .take(missing_tokens.len().saturating_sub(1))
            {
                if normalized_name.contains(token.as_str())
                    || token.contains(normalized_name.as_str())
                {
                    score = score.saturating_add(6);
                }
            }
        }

        if score == 0 {
            continue;
        }
        matched_return_fields.sort();
        matched_return_fields.dedup();
        matches.push(MissingFactQueryCandidate {
            query_ref: query_ref.to_string(),
            score,
            matched_return_fields,
        });
    }

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.query_ref.cmp(&right.query_ref))
    });
    let truncated = matches.len() > limit;
    matches.truncate(limit);
    (matches, truncated)
}

fn query_return_field_names(detail: &Value) -> Vec<String> {
    detail
        .get("returns")
        .and_then(Value::as_array)
        .map(|returns| {
            returns
                .iter()
                .filter_map(|entry| entry.get("name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn normalize_missing_fact_ref(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | ',' | ';' | '.' | ')' | '(')
    });
    let right_of_equals = trimmed
        .rsplit_once('=')
        .map(|(_, value)| value)
        .unwrap_or(trimmed);
    let normalized = right_of_equals
        .strip_prefix("runtime.")
        .unwrap_or(right_of_equals);
    let key = if let Some(key) = normalized.strip_prefix("inputs.") {
        key
    } else if let Some(key) = normalized.strip_prefix("input.") {
        key
    } else {
        normalized
    };
    let key = key.strip_suffix(".value").unwrap_or(key).trim_matches('.');
    if key.is_empty() {
        return None;
    }
    Some(format!("inputs.{key}"))
}

fn normalized_tokens(raw: &str) -> Vec<String> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let normalized = normalized_identifier(token);
            (!normalized.is_empty()).then_some(normalized)
        })
        .collect::<Vec<_>>()
}

fn normalized_identifier(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn requires_successful_check_before_finalize(
    phase: PlannerRoundPhase,
    segment_check_context: Option<&SegmentCheckContext>,
) -> bool {
    super::tools::check_segment::requires_successful_check_before_finalize(
        phase,
        segment_check_context.is_some(),
    )
}

fn missing_pre_finalize_check_payload(finalize_tool: &str) -> Value {
    super::tools::check_segment::missing_pre_finalize_check_payload(finalize_tool)
}

fn pre_finalize_segment_mismatch_payload(
    finalize_tool: &str,
    checked_signature: Option<&str>,
    finalized_signature: Option<&str>,
) -> Value {
    super::tools::check_segment::pre_finalize_segment_mismatch_payload(
        finalize_tool,
        checked_signature,
        finalized_signature,
    )
}

struct FinalizeSchemaRepairPayload {
    payload: Value,
    sub_reason_code: &'static str,
}

struct NonFinalizeToolArgsRepairPayload {
    payload: Value,
    sub_reason_code: &'static str,
}

fn finalize_schema_repair_payload(
    error: &RunnerError,
    finalize_tool: &str,
    round: u8,
    attempt: u8,
    max_attempts: u8,
) -> Option<FinalizeSchemaRepairPayload> {
    let RunnerError::Llm(message) = error else {
        return None;
    };
    if !message.contains(format!("invalid {finalize_tool} args").as_str()) {
        return None;
    }

    if message.contains("missing field `status`") {
        return Some(FinalizeSchemaRepairPayload {
            sub_reason_code: "missing_status",
            payload: json!({
                "error": {
                    "code": "finalize_schema_error",
                    "reason_code": "schema_missing_required_field",
                    "sub_reason_code": "missing_status",
                    "phase_reason_code": "planning.schema_missing_required_field",
                    "message": format!("`{finalize_tool}` output is missing required field `status`"),
                    "tool": finalize_tool,
                    "round": round,
                    "repair_attempt": attempt,
                    "max_repair_attempts": max_attempts,
                    "required_fields": ["status", "done"],
                    "allowed_status": ["proposed", "invalid", "unavailable"],
                    "contract": {
                        "proposed": {"required": ["status", "done", "segment"]},
                        "invalid_or_unavailable": {"required": ["status", "done", "error.reason_code"]}
                    }
                }
            }),
        });
    }

    if message.contains(
        "status=proposed with ready_for_todos=false requires non-empty `questions` or `missing_refs`",
    ) {
        return Some(FinalizeSchemaRepairPayload {
            sub_reason_code: "grounding_not_ready_non_actionable",
            payload: json!({
                "error": {
                    "code": "finalize_schema_error",
                    "reason_code": "schema_missing_required_field",
                    "sub_reason_code": "grounding_not_ready_non_actionable",
                    "phase_reason_code": "planning.grounding_not_ready_non_actionable",
                    "message": "`plan.ground_intent` with status=proposed and ready_for_todos=false must include non-empty `questions` or `missing_refs`",
                    "tool": finalize_tool,
                    "round": round,
                    "repair_attempt": attempt,
                    "max_repair_attempts": max_attempts,
                    "required_any_of": [
                        {"questions": "non-empty array"},
                        {"missing_refs": "non-empty array"}
                    ],
                    "examples": {
                        "good": [
                            {"status":"proposed","ready_for_todos":false,"questions":[{"id":"inputs.owner","question":"What owner address should be used?"}]},
                            {"status":"proposed","ready_for_todos":false,"missing_refs":["inputs.token.decimals"]}
                        ],
                        "bad": [
                            {"status":"proposed","ready_for_todos":false},
                            {"status":"proposed","ready_for_todos":false,"questions":[],"missing_refs":[]}
                        ]
                    }
                }
            }),
        });
    }

    if message.contains("invalid type:") {
        let expected_bool = message.contains("expected a boolean")
            || message.contains("expected boolean")
            || message.contains("expected `bool`");
        let (sub_reason_code, message_text, expected_type) = if expected_bool {
            (
                "invalid_boolean_type",
                format!(
                    "`{finalize_tool}` output has a boolean type mismatch; ensure `done` is a JSON boolean (true/false), not a string"
                ),
                "boolean",
            )
        } else {
            (
                "invalid_type",
                format!(
                    "`{finalize_tool}` output has one or more schema type mismatches; ensure all finalize fields match JSON schema types"
                ),
                "schema-defined",
            )
        };
        return Some(FinalizeSchemaRepairPayload {
            sub_reason_code,
            payload: json!({
                "error": {
                    "code": "finalize_schema_error",
                    "reason_code": "schema_invalid_type",
                    "sub_reason_code": sub_reason_code,
                    "phase_reason_code": format!("planning.{sub_reason_code}"),
                    "message": message_text,
                    "raw_error": message,
                    "tool": finalize_tool,
                    "round": round,
                    "repair_attempt": attempt,
                    "max_repair_attempts": max_attempts,
                    "expected_type": expected_type,
                    "typing_examples": {
                        "good": [{"done": false}, {"done": true}],
                        "bad": [{"done": "false"}, {"done": "true"}]
                    },
                    "required_fields": ["status", "done"],
                    "allowed_status": ["proposed", "invalid", "unavailable"]
                }
            }),
        });
    }

    None
}

fn non_finalize_tool_args_repair_payload(
    error: &RunnerError,
    tool_name: &str,
    round: u8,
    attempt: u8,
    max_attempts: u8,
) -> Option<NonFinalizeToolArgsRepairPayload> {
    let RunnerError::Llm(message) = error else {
        return None;
    };
    if tool_name != "plan.check_segment" || !message.contains("invalid plan.check_segment args") {
        return None;
    }

    if message.contains("missing field `segment`") {
        return Some(NonFinalizeToolArgsRepairPayload {
            sub_reason_code: "missing_segment",
            payload: json!({
                "error": {
                    "code": "tool_args_schema_error",
                    "reason_code": "schema_missing_required_field",
                    "sub_reason_code": "missing_segment",
                    "phase_reason_code": "planning.schema_missing_required_field",
                    "message": "`plan.check_segment` args must include root field `segment` (object)",
                    "tool": tool_name,
                    "round": round,
                    "repair_attempt": attempt,
                    "max_repair_attempts": max_attempts,
                    "required_fields": ["segment"],
                    "shape": {
                        "required_root": {"segment": "<segment_object>"},
                        "good": {
                            "segment": {
                                "segment_id": "seg_1",
                                "cursor_in": "0",
                                "cursor_out": "1",
                                "done": false,
                                "steps": []
                            }
                        },
                        "bad": {"raw": "{\"segment\": {...}}"}
                    }
                }
            }),
        });
    }

    if message.contains("invalid type:") {
        return Some(NonFinalizeToolArgsRepairPayload {
            sub_reason_code: "invalid_segment_type",
            payload: json!({
                "error": {
                    "code": "tool_args_schema_error",
                    "reason_code": "schema_invalid_type",
                    "sub_reason_code": "invalid_segment_type",
                    "phase_reason_code": "planning.schema_invalid_type",
                    "message": "`plan.check_segment` args type mismatch: `segment` must be a JSON object matching ais-plan-sketch segment schema",
                    "raw_error": message,
                    "tool": tool_name,
                    "round": round,
                    "repair_attempt": attempt,
                    "max_repair_attempts": max_attempts,
                    "typing_examples": {
                        "good": [{"segment":{"segment_id":"seg_1","cursor_in":"0","cursor_out":"1","done":false,"steps":[]}}],
                        "bad": [{"segment":"{...}"}, {"segment":123}, {"raw":"{\"segment\":{...}}"}]
                    }
                }
            }),
        });
    }

    Some(NonFinalizeToolArgsRepairPayload {
        sub_reason_code: "invalid_tool_args",
        payload: json!({
            "error": {
                "code": "tool_args_schema_error",
                "reason_code": "schema_invalid_arguments",
                "sub_reason_code": "invalid_tool_args",
                "phase_reason_code": "planning.schema_invalid_arguments",
                "message": "`plan.check_segment` args are invalid; provide {\"segment\": <object>} and ensure all fields use schema-correct JSON types.",
                "raw_error": message,
                "tool": tool_name,
                "round": round,
                "repair_attempt": attempt,
                "max_repair_attempts": max_attempts
            }
        }),
    })
}

fn plan_check_result_ok(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|value| value.get("ok").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn plan_check_segment_signature_from_tool_args(arguments: &Value) -> Option<String> {
    let segment = arguments.get("segment")?;
    let decoded = decode_plan_sketch_segment_arg(segment).ok()?;
    plan_sketch_segment_signature(&decoded)
}

fn finalized_segment_signature(result: &PlannerToolOutput) -> Option<String> {
    let PlannerToolOutput::SegmentDraft(SegmentDraft::Proposed { segment, .. }) = result else {
        return None;
    };
    plan_sketch_segment_signature(segment)
}

fn plan_sketch_segment_signature(segment: &PlanSketchSegment) -> Option<String> {
    let value = serde_json::to_value(segment).ok()?;
    stable_hash_hex(&value, &StableJsonOptions::default()).ok()
}

fn plan_check_failure_signature(content: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(content).ok()?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let reason_code = value
        .get("reason_code")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/reason_code").and_then(Value::as_str))
        .unwrap_or("compile_error");
    let mut issue_keys = plan_check_issue_summaries(&value)
        .into_iter()
        .map(|issue| format!("{}@{}", issue.reason_code, issue.step_id))
        .collect::<Vec<_>>();
    issue_keys.sort();
    issue_keys.dedup();
    Some(format!("{reason_code}|{}", issue_keys.join(",")))
}

fn repeated_plan_check_failure_payload(
    content: &str,
    streak: u64,
    threshold: u64,
    finalize_tool: &str,
) -> Value {
    super::tools::check_segment::repeated_plan_check_failure_payload(
        content,
        streak,
        threshold,
        finalize_tool,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanCheckIssueSummary {
    reason_code: String,
    step_id: String,
    message: String,
    reference: String,
    path: String,
    suggested_ref: Option<String>,
    candidates: Vec<String>,
}

fn plan_check_issue_summaries(value: &Value) -> Vec<PlanCheckIssueSummary> {
    let issues = value
        .get("issues")
        .and_then(Value::as_array)
        .or_else(|| value.pointer("/error/issues").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    issues
        .iter()
        .map(|item| PlanCheckIssueSummary {
            reason_code: item
                .get("gate_reason_code")
                .and_then(Value::as_str)
                .or_else(|| item.get("reason_code").and_then(Value::as_str))
                .unwrap_or("unknown")
                .to_string(),
            step_id: item
                .get("step_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: item
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            reference: item
                .get("reference")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            path: item
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            suggested_ref: item
                .get("suggested_ref")
                .and_then(Value::as_str)
                .map(str::to_string),
            candidates: item
                .get("candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|items| items.iter())
                .filter_map(Value::as_str)
                .take(3)
                .map(str::to_string)
                .collect::<Vec<_>>(),
        })
        .collect::<Vec<_>>()
}

fn plan_check_has_control_step_candidate_not_found(content: &str) -> bool {
    super::tools::check_segment::plan_check_has_control_step_candidate_not_found(content)
}

fn finalized_segment_is_proposed(result: &PlannerToolOutput) -> bool {
    matches!(
        result,
        PlannerToolOutput::SegmentDraft(SegmentDraft::Proposed { .. })
    )
}

fn workspace_summary(candidate_context: Option<&CandidateContext>) -> Value {
    let Some(context) = candidate_context else {
        return json!({
            "available": false,
            "reason": "workspace candidate context unavailable"
        });
    };
    json!({
        "available": true,
        "actions": context.executable_candidates.actions.len(),
        "queries": context.executable_candidates.queries.len(),
        "execution_plugins": context.executable_candidates.execution_plugins.len(),
        "protocols": context.protocols.len(),
        "chain_scope": context.executable_candidates.chain_scope,
    })
}

fn workspace_summary_lines(summary: &Value) -> String {
    let Some(obj) = summary.as_object() else {
        return "- unavailable".to_string();
    };
    let mut lines = Vec::<String>::new();
    let mut keys = obj.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(value) = obj.get(key.as_str()) else {
            continue;
        };
        lines.push(format!("- {}: {}", key, compact_value_for_prompt(value)));
    }
    lines.join("\n")
}

fn compact_value_for_prompt(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(v) => {
            let rendered = v
                .iter()
                .take(4)
                .map(compact_value_for_prompt)
                .collect::<Vec<_>>();
            if v.len() > 4 {
                format!("[{}, ...]", rendered.join(", "))
            } else {
                format!("[{}]", rendered.join(", "))
            }
        }
        Value::Object(_) => value.to_string(),
    }
}

fn numbered_lines(lines: &[String]) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}. {}", index + 1, line.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn non_empty_rules(lines: Option<Vec<String>>) -> Option<Vec<String>> {
    let lines = lines?
        .into_iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    (!lines.is_empty()).then_some(lines)
}

pub(super) fn coerce_required_scalar_string(
    field: &str,
    value: &Value,
) -> Result<String, RunnerError> {
    let parsed = match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        _ => {
            return Err(RunnerError::Llm(format!(
                "plan.begin `{field}` must be a non-empty scalar (string/number/bool)"
            )));
        }
    };
    if parsed.is_empty() {
        return Err(RunnerError::Llm(format!(
            "plan.begin `{field}` must be non-empty"
        )));
    }
    Ok(parsed)
}

pub(super) fn parse_segment_draft(args: SegmentToolArgs) -> Result<SegmentDraft, RunnerError> {
    let SegmentToolArgs {
        status,
        done,
        segment,
        summary,
        cursor_next,
        issues,
        error,
        questions,
    } = args;
    match status.as_str() {
        "proposed" => {
            let segment_raw = segment.ok_or_else(|| {
                RunnerError::Llm("proposed segment draft requires `segment`".to_string())
            })?;
            let segment = decode_plan_sketch_segment_arg(&segment_raw)?;
            let cursor_next = match cursor_next {
                Some(cursor_next_raw) => {
                    coerce_required_scalar_string("cursor_next", &cursor_next_raw)?
                }
                None => segment.cursor_out.clone(),
            };
            Ok(SegmentDraft::Proposed {
                summary,
                segment,
                cursor_next,
                done,
                issues: issues.into_values(),
            })
        }
        "unavailable" => {
            let error = error.ok_or_else(|| {
                RunnerError::Llm("unavailable segment draft requires `error`".to_string())
            })?;
            let questions = extract_missing_input_questions(error.details.as_ref(), &questions);
            validate_missing_required_input_error("plan.propose_segment|plan.revise_segment", &error, questions.as_slice())?;
            Ok(SegmentDraft::Unavailable {
                reason_code: error.reason_code,
                message: error.message,
                done,
                issues: issues.into_values(),
                questions,
            })
        }
        "invalid" => {
            let error = error.ok_or_else(|| {
                RunnerError::Llm("invalid segment draft requires `error`".to_string())
            })?;
            Ok(SegmentDraft::Invalid {
                reason_code: error.reason_code,
                message: error.message,
                done,
                issues: issues.into_values(),
            })
        }
        other => Err(RunnerError::Llm(format!(
            "invalid segment draft status `{other}`"
        ))),
    }
}

pub(super) fn parse_todo_draft(args: TodoToolArgs) -> Result<TodoDraft, RunnerError> {
    let TodoToolArgs {
        status,
        todos,
        summary,
        issues,
        error,
        questions,
    } = args;
    match status.as_str() {
        "proposed" => {
            if todos.is_empty() {
                return Err(RunnerError::Llm(
                    "proposed todo draft requires non-empty `todos`".to_string(),
                ));
            }
            Ok(TodoDraft::Proposed {
                summary,
                todos,
                issues: issues.into_values(),
            })
        }
        "unavailable" => {
            let error = error.ok_or_else(|| {
                RunnerError::Llm("unavailable todo draft requires `error`".to_string())
            })?;
            let questions = extract_missing_input_questions(error.details.as_ref(), &questions);
            validate_missing_required_input_error("plan.propose_todos", &error, questions.as_slice())?;
            Ok(TodoDraft::Unavailable {
                reason_code: error.reason_code,
                message: error.message,
                issues: issues.into_values(),
                questions,
            })
        }
        "invalid" => {
            let error = error.ok_or_else(|| {
                RunnerError::Llm("invalid todo draft requires `error`".to_string())
            })?;
            Ok(TodoDraft::Invalid {
                reason_code: error.reason_code,
                message: error.message,
                issues: issues.into_values(),
            })
        }
        other => Err(RunnerError::Llm(format!(
            "invalid todo draft status `{other}`"
        ))),
    }
}

pub(super) fn parse_grounding_draft(
    args: GroundingToolArgs,
) -> Result<IntentGroundingDraft, RunnerError> {
    let GroundingToolArgs {
        status,
        summary,
        ready_for_todos,
        resolved_inputs,
        intent_facts,
        confidence,
        issues,
        error,
        questions,
        missing_refs,
    } = args;
    match status.as_str() {
        "proposed" => {
            let issues = issues.into_values();
            let questions = questions.into_values();
            let has_actionable_missing_refs = missing_refs
                .iter()
                .any(|missing_ref| !missing_ref.trim().is_empty());
            if ready_for_todos == Some(false)
                && questions.is_empty()
                && !has_actionable_missing_refs
            {
                return Err(RunnerError::Llm(
                    "invalid plan.ground_intent args: status=proposed with ready_for_todos=false requires non-empty `questions` or `missing_refs`".to_string(),
                ));
            }
            let inferred_ready = ready_for_todos
                .unwrap_or_else(|| questions.is_empty() && !resolved_inputs.is_empty());
            Ok(IntentGroundingDraft::Proposed {
                summary,
                ready_for_todos: inferred_ready,
                resolved_inputs,
                intent_facts,
                confidence,
                issues,
                questions,
            })
        }
        "unavailable" => {
            let error = error.ok_or_else(|| {
                RunnerError::Llm("unavailable grounding draft requires `error`".to_string())
            })?;
            let questions = extract_missing_input_questions(error.details.as_ref(), &questions);
            validate_missing_required_input_error("plan.ground_intent", &error, questions.as_slice())?;
            Ok(IntentGroundingDraft::Unavailable {
                reason_code: error.reason_code,
                message: error.message,
                issues: issues.into_values(),
                questions,
            })
        }
        "invalid" => {
            let error = error.ok_or_else(|| {
                RunnerError::Llm("invalid grounding draft requires `error`".to_string())
            })?;
            Ok(IntentGroundingDraft::Invalid {
                reason_code: error.reason_code,
                message: error.message,
                issues: issues.into_values(),
            })
        }
        other => Err(RunnerError::Llm(format!(
            "invalid grounding draft status `{other}`"
        ))),
    }
}

pub(super) fn decode_grounding_tool_args(
    raw_arguments: Value,
    tool_name: &str,
) -> Result<GroundingToolArgs, RunnerError> {
    let normalized = normalize_grounding_tool_arguments(raw_arguments);
    decode_planner_finalize_tool_args(normalized, tool_name)
}

pub(super) fn decode_segment_tool_args(
    raw_arguments: Value,
    tool_name: &str,
) -> Result<SegmentToolArgs, RunnerError> {
    decode_planner_finalize_tool_args(raw_arguments, tool_name)
}

pub(super) fn decode_todo_tool_args(
    raw_arguments: Value,
    tool_name: &str,
) -> Result<TodoToolArgs, RunnerError> {
    decode_planner_finalize_tool_args(raw_arguments, tool_name)
}

fn decode_planner_finalize_tool_args<T>(
    raw_arguments: Value,
    tool_name: &str,
) -> Result<T, RunnerError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(raw_arguments)
        .map_err(|error| RunnerError::Llm(format!("invalid {tool_name} args: {error}")))
}

fn normalize_grounding_tool_arguments(raw_arguments: Value) -> Value {
    let Some(object) = raw_arguments.as_object() else {
        return raw_arguments;
    };
    let mut out = object.clone();

    // Some providers/models collapse the whole payload into a JSON string under
    // `intent_facts`; unpack it when that string looks like a full grounding payload.
    if let Some(parsed_intent_facts) = out
        .get("intent_facts")
        .and_then(parse_stringified_json_value)
    {
        if let Some(payload) = parsed_intent_facts.as_object() {
            let looks_like_payload = payload.contains_key("status")
                || payload.contains_key("ready_for_todos")
                || payload.contains_key("resolved_inputs")
                || payload.contains_key("intent_facts");
            if looks_like_payload {
                for key in [
                    "status",
                    "summary",
                    "ready_for_todos",
                    "resolved_inputs",
                    "intent_facts",
                    "confidence",
                    "issues",
                    "error",
                    "questions",
                ] {
                    if !out.contains_key(key) {
                        if let Some(value) = payload.get(key) {
                            out.insert(key.to_string(), value.clone());
                        }
                    }
                }
                if let Some(intent_facts) = payload.get("intent_facts") {
                    out.insert("intent_facts".to_string(), intent_facts.clone());
                }
            } else {
                out.insert("intent_facts".to_string(), parsed_intent_facts);
            }
        } else {
            out.insert("intent_facts".to_string(), parsed_intent_facts);
        }
    }

    coerce_json_string_field(&mut out, "resolved_inputs");
    coerce_json_string_field(&mut out, "intent_facts");
    coerce_json_string_field(&mut out, "confidence");
    coerce_json_string_field(&mut out, "issues");
    coerce_json_string_field(&mut out, "questions");
    coerce_json_string_field(&mut out, "error");
    coerce_bool_string_field(&mut out, "ready_for_todos");

    Value::Object(out)
}

fn parse_stringified_json_value(raw: &Value) -> Option<Value> {
    raw.as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
}

fn coerce_json_string_field(target: &mut serde_json::Map<String, Value>, key: &str) {
    let Some(raw) = target.get(key).cloned() else {
        return;
    };
    let Some(parsed) = parse_stringified_json_value(&raw) else {
        return;
    };
    target.insert(key.to_string(), parsed);
}

fn coerce_bool_string_field(target: &mut serde_json::Map<String, Value>, key: &str) {
    let Some(Value::String(raw)) = target.get(key) else {
        return;
    };
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "true" | "1" => {
            target.insert(key.to_string(), Value::Bool(true));
        }
        "false" | "0" => {
            target.insert(key.to_string(), Value::Bool(false));
        }
        _ => {}
    }
}

fn extract_missing_input_questions(
    details: Option<&PlannerErrorDetails>,
    fallback: &PlannerQuestionList,
) -> Vec<Value> {
    if !fallback.is_empty() {
        return fallback.to_values();
    }
    details
        .map(PlannerErrorDetails::missing_input_questions)
        .unwrap_or_default()
        .iter()
        .filter_map(|question| serde_json::to_value(question).ok())
        .collect::<Vec<_>>()
}

fn validate_missing_required_input_error(
    tool_name: &str,
    error: &PlannerToolError,
    questions: &[Value],
) -> Result<(), RunnerError> {
    if error.reason_code != "missing_required_input" {
        return Ok(());
    }
    if questions.is_empty() {
        return Err(RunnerError::Llm(format!(
            "invalid {tool_name} args: reason_code=missing_required_input requires non-empty error.details.questions[]"
        )));
    }
    if let Some(invalid_question_id) = questions.iter().find_map(|question| {
        question
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| id.starts_with("params."))
    }) {
        return Err(RunnerError::Llm(format!(
            "invalid {tool_name} args: missing_required_input question.id must be canonical source ref (received `{invalid_question_id}`)"
        )));
    }
    if let Some(invalid_unresolved_ref) = error
        .details
        .as_ref()
        .into_iter()
        .flat_map(PlannerErrorDetails::recovery_exhaustion_unresolved_refs)
        .map(|item| item.trim().to_string())
        .find(|reference| reference.starts_with("params."))
    {
        return Err(RunnerError::Llm(format!(
            "invalid {tool_name} args: missing_required_input recovery_exhaustion.unresolved_refs must use source refs only (received `{invalid_unresolved_ref}`)"
        )));
    }
    let reasons = error
        .details
        .as_ref()
        .into_iter()
        .flat_map(PlannerErrorDetails::recovery_exhaustion_reasons)
        .collect::<Vec<_>>();
    if reasons.is_empty() {
        return Err(RunnerError::Llm(format!(
            "invalid {tool_name} args: missing_required_input requires non-empty error.details.recovery_exhaustion.reasons[]"
        )));
    }
    if error
        .details
        .as_ref()
        .and_then(PlannerErrorDetails::recovery_exhaustion_attempt_trace_id)
        .is_none()
    {
        return Err(RunnerError::Llm(format!(
            "invalid {tool_name} args: missing_required_input requires non-empty error.details.recovery_exhaustion.attempt_trace_id"
        )));
    }
    Ok(())
}

pub(super) fn decode_plan_sketch_segment_arg(
    raw: &Value,
) -> Result<PlanSketchSegment, RunnerError> {
    let mut value = if let Some(raw_text) = raw.as_str() {
        let parsed: Value = serde_json::from_str(raw_text).map_err(|error| {
            RunnerError::Llm(format!(
                "proposed segment draft `segment` string must be valid JSON object text: {error}"
            ))
        })?;
        if !parsed.is_object() {
            return Err(RunnerError::Llm(
                "proposed segment draft `segment` must decode to a JSON object".to_string(),
            ));
        }
        parsed
    } else {
        raw.clone()
    };
    if let Some(details) = missing_step_candidate_ref_diagnostics(&value) {
        return Err(RunnerError::Llm(format!(
            "proposed segment draft `segment` is invalid: steps missing required `candidate_ref`: {details}. Only query/action steps require candidate_ref."
        )));
    }
    ensure_step_inputs_field(&mut value);
    serde_json::from_value::<PlanSketchSegment>(value).map_err(|error| {
        RunnerError::Llm(format!(
            "proposed segment draft `segment` must be a valid PlanSketchSegment: {error}"
        ))
    })
}

fn ensure_step_inputs_field(raw: &mut Value) {
    let Some(steps) = raw
        .as_object_mut()
        .and_then(|segment| segment.get_mut("steps"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for step in steps {
        let Some(step_obj) = step.as_object_mut() else {
            continue;
        };
        if !step_obj.contains_key("inputs") {
            step_obj.insert("inputs".to_string(), Value::Object(serde_json::Map::new()));
        }
    }
}

fn missing_step_candidate_ref_diagnostics(raw: &Value) -> Option<String> {
    let steps = raw.get("steps").and_then(Value::as_array)?;
    let mut missing = Vec::<String>::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step_obj) = step.as_object() else {
            continue;
        };
        let kind = step_obj
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if kind == "assert" || kind == "branch" {
            continue;
        }
        if step_obj.contains_key("candidate_ref") {
            continue;
        }
        let step_id = step_obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("#{index}"));
        let step_kind = step_obj
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "unknown".to_string());
        missing.push(format!("{step_id}({step_kind})"));
    }
    if missing.is_empty() {
        return None;
    }
    if missing.len() > 6 {
        let extra = missing.len().saturating_sub(6);
        missing.truncate(6);
        missing.push(format!("...+{extra}"));
    }
    Some(missing.join(", "))
}

fn segmented_planner_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list_candidates".to_string(),
            description: "Get executable candidates snapshot for current workspace/pack"
                .to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{
                "chain":{"type":"string"},
                "protocol":{"type":"string"},
                "filter":{
                  "type":"object",
                  "properties":{
                    "chain":{"type":"string"},
                    "protocol":{"type":"string"}
                  },
                  "additionalProperties":false
                }
              },
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "get_candidate_detail".to_string(),
            description: "Fetch candidate detail cards by refs".to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{"refs":{"type":"array","items":{"type":"string"}}},
              "required":["refs"],
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "catalog.search".to_string(),
            description: "Search candidates by keyword/risk/chain with bounded result size"
                .to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{
                "query":{"type":"string"},
                "kind":{"type":"string","enum":["action","query","any"]},
                "chain":{"type":"string"},
                "min_risk_level":{"type":"integer","minimum":0,"maximum":10},
                "max_risk_level":{"type":"integer","minimum":0,"maximum":10},
                "limit":{"type":"integer","minimum":1,"maximum":24}
              },
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "catalog.resolve_missing_facts".to_string(),
            description: "Resolve missing input facts to query candidates by protocol query returns."
                .to_string(),
            input_schema: catalog_resolve_missing_facts_tool_schema(),
        },
        ToolSpec {
            name: "guide.get".to_string(),
            description: "Lookup schema/topic guides. Request shape must be exactly one of {schema:\"ais-plan-sketch/0.1.0\"} or {topic:\"cel\"}; optional {full:true} returns full schema JSON for schema requests.".to_string(),
            input_schema: json!({
              "type":"object",
              "oneOf":[
                {
                  "properties":{
                    "schema":{"type":"string","minLength":1},
                    "full":{"type":"boolean"}
                  },
                  "required":["schema"],
                  "not":{"required":["topic"]}
                },
                {
                  "properties":{
                    "topic":{"type":"string","enum":["cel","valueref"]},
                    "full":{"type":"boolean"}
                  },
                  "required":["topic"],
                  "not":{"required":["schema"]}
                }
              ],
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "plan.check_segment".to_string(),
            description: "Compile-check a segment and return structured issues without executing."
                .to_string(),
            input_schema: plan_check_segment_tool_schema(),
        },
        ToolSpec {
            name: "plan.begin".to_string(),
            description:
                "Initialize planning session and return session_id/snapshot_hash/cursor/limits."
                    .to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{
                "session_id":{"type":"string"},
                "snapshot_hash":{"type":"string"},
                "cursor":{"oneOf":[{"type":"string"},{"type":"integer"},{"type":"number"}]},
                "limits":{
                  "type":"object",
                  "properties":{
                    "max_rounds":{"type":"integer","minimum":1},
                    "max_segments":{"type":"integer","minimum":1}
                  },
                  "required":["max_rounds","max_segments"],
                  "additionalProperties":false
                }
              },
              "required":["session_id","snapshot_hash","cursor","limits"],
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "plan.ground_intent".to_string(),
            description: "Ground intent into initial inputs/facts and readiness for todo planning."
                .to_string(),
            input_schema: plan_ground_intent_tool_schema(),
        },
        ToolSpec {
            name: "plan.propose_todos".to_string(),
            description: "Plan deterministic todo list for the intent before segment execution."
                .to_string(),
            input_schema: plan_propose_todos_tool_schema(),
        },
        ToolSpec {
            name: "plan.propose_segment".to_string(),
            description: "Propose next segment from cursor and state summary.".to_string(),
            input_schema: plan_propose_segment_tool_schema(),
        },
        ToolSpec {
            name: "plan.revise_segment".to_string(),
            description: "Revise current segment after compile/runtime failure.".to_string(),
            input_schema: plan_revise_segment_tool_schema(),
        },
    ]
}

fn segmented_planner_tools_for_phase(phase: PlannerRoundPhase) -> Vec<ToolSpec> {
    segmented_planner_tools()
        .into_iter()
        .filter(|tool| ensure_tool_allowed_for_phase(tool.name.as_str(), phase).is_ok())
        .collect::<Vec<_>>()
}

fn plan_propose_segment_tool_schema() -> Value {
    agent_planning_tools_payload_schema("segment_payload")
}

fn plan_propose_todos_tool_schema() -> Value {
    agent_planning_tools_payload_schema("todos_payload")
}

fn catalog_resolve_missing_facts_tool_schema() -> Value {
    json!({
      "type":"object",
      "properties":{
        "missing_refs":{
          "type":"array",
          "items":{"type":"string","minLength":1},
          "minItems":1
        },
        "limit_per_ref":{"type":"integer","minimum":1,"maximum":8}
      },
      "required":["missing_refs"],
      "additionalProperties":false
    })
}

fn plan_ground_intent_tool_schema() -> Value {
    json!({
      "type":"object",
      "additionalProperties": false,
      "required":["session_id","snapshot_hash","cursor","status"],
      "properties":{
        "session_id":{"type":"string","minLength":1},
        "snapshot_hash":{"type":"string","pattern":"^[0-9a-f]{64}$"},
        "cursor":{"type":"string","minLength":1},
        "state_summary":{"type":"object"},
        "status":{"type":"string","enum":["proposed","unavailable","invalid"]},
        "summary":{"type":"string"},
        "ready_for_todos":{"type":"boolean"},
        "resolved_inputs":{"type":"object","additionalProperties":true},
        "intent_facts":{"type":"object","additionalProperties":true},
        "confidence":{"type":"object","additionalProperties":{"type":"integer","minimum":0,"maximum":100}},
        "issues":{"type":"array","items":{"type":"object"}},
        "questions":{"type":"array","items":{"$ref":"#/definitions/missing_input_question"}},
        "missing_refs":{"type":"array","items":{"type":"string","minLength":1},"minItems":1},
        "error":{"$ref":"#/definitions/error"}
      },
      "allOf":[
        {
          "if":{"properties":{"status":{"const":"proposed"}},"required":["status"]},
          "then":{"required":["ready_for_todos"]}
        },
        {
          "if":{"properties":{"status":{"const":"proposed"},"ready_for_todos":{"const":false}},"required":["status","ready_for_todos"]},
          "then":{
            "anyOf":[
              {"required":["questions"],"properties":{"questions":{"minItems":1}}},
              {"required":["missing_refs"],"properties":{"missing_refs":{"minItems":1}}}
            ]
          }
        },
        {
          "if":{"properties":{"status":{"enum":["unavailable","invalid"]}},"required":["status"]},
          "then":{"required":["error"]}
        }
      ],
      "definitions":{
        "missing_input_option":{
          "type":"object",
          "additionalProperties":false,
          "required":["label"],
          "properties":{
            "value":{},
            "label":{"type":"string","minLength":1},
            "description":{"type":"string"}
          }
        },
        "missing_input_question":{
          "type":"object",
          "additionalProperties":false,
          "required":["id","question"],
          "properties":{
            "id":{"type":"string","minLength":1},
            "question":{"type":"string","minLength":1},
            "options":{"type":"array","items":{"$ref":"#/definitions/missing_input_option"}},
            "required":{"type":"boolean"}
          }
        },
        "error":{
          "type":"object",
          "additionalProperties":false,
          "required":["reason_code"],
          "properties":{
            "reason_code":{"type":"string","minLength":1},
            "message":{"type":"string"},
            "details":{"type":"object","additionalProperties":true}
          }
        }
      }
    })
}

fn plan_revise_segment_tool_schema() -> Value {
    // Finalize output contract is identical between propose/revise. `revise_payload`
    // includes host-side context fields (for example `previous_error`) that are not
    // part of the finalize tool output and would over-constrain model responses.
    agent_planning_tools_payload_schema("segment_payload")
}

fn plan_check_segment_tool_schema() -> Value {
    let embedded = get_json_schema(SCHEMA_PLAN_SKETCH_0_1_0)
        .unwrap_or_else(|| panic!("schema `{SCHEMA_PLAN_SKETCH_0_1_0}` must exist"));
    let root: Value = serde_json::from_str(embedded.json).unwrap_or_else(|error| {
        panic!("schema `{SCHEMA_PLAN_SKETCH_0_1_0}` must be valid JSON: {error}")
    });
    let mut segment = root
        .pointer("/$defs/segment")
        .cloned()
        .unwrap_or_else(|| panic!("schema `{SCHEMA_PLAN_SKETCH_0_1_0}` missing `/$defs/segment`"));
    if let (Some(defs), Some(segment_obj)) = (root.get("$defs"), segment.as_object_mut()) {
        segment_obj.insert("$defs".to_string(), defs.clone());
    }
    json!({
      "type":"object",
      "properties":{
        "segment": segment
      },
      "required":["segment"],
      "additionalProperties": false
    })
}

fn agent_planning_tools_payload_schema(definition: &str) -> Value {
    let embedded = get_json_schema(SCHEMA_AGENT_PLANNING_TOOLS_0_1_0)
        .unwrap_or_else(|| panic!("schema `{SCHEMA_AGENT_PLANNING_TOOLS_0_1_0}` must exist"));
    let root: Value = serde_json::from_str(embedded.json).unwrap_or_else(|error| {
        panic!("schema `{SCHEMA_AGENT_PLANNING_TOOLS_0_1_0}` must be valid JSON: {error}")
    });
    let mut payload = root
        .pointer(&format!("/$defs/{definition}"))
        .cloned()
        .unwrap_or_else(|| {
            panic!("schema `{SCHEMA_AGENT_PLANNING_TOOLS_0_1_0}` missing `/$defs/{definition}`")
        });
    if let (Some(defs), Some(payload_obj)) = (root.get("$defs"), payload.as_object_mut()) {
        payload_obj.insert("$defs".to_string(), defs.clone());
    }
    payload
}

fn render_begin_prompt_with_patch(request: &SegmentBeginRequest, patch: Option<&Value>) -> String {
    let mut payload = json!({
        "schema": "ais-agent-intent/0.0.1",
        "intent": request.intent,
        "begin_contract": {
            "required_fields": ["session_id", "snapshot_hash", "cursor", "limits.max_rounds", "limits.max_segments"],
            "cursor_type": "string_or_number",
            "snapshot_hash_rule": "must echo the provided snapshot_hash exactly",
            "note": "cursor will be normalized to string by runner"
        },
        "snapshot_hash": request.snapshot_hash,
        "pack_snapshot_hash": request.pack_snapshot_hash,
        "catalog_hash": request.catalog_hash,
        "chain_scope": request.chain_scope,
        "goal": "call plan.begin"
    });
    if let Some(patch) = patch {
        merge_json_patch(&mut payload, patch);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn render_todos_prompt_with_patch(request: &TodoPlanningRequest, patch: Option<&Value>) -> String {
    let prompt_state_summary = state_summary_for_prompt(request.state_summary.as_ref());
    let mut payload = json!({
        "schema": "ais-agent-intent/0.0.1",
        "intent": request.intent,
        "tool": "plan.propose_todos",
        "todo_contract": {
            "required_fields": ["title"],
            "optional_fields": ["required_facts", "produced_facts", "acceptance"],
            "status_enum": ["proposed", "unavailable", "invalid"],
            "rules": [
                "Return status=proposed with a non-empty todos array when intent is actionable.",
                "Todos must be deterministic, concise, and non-overlapping.",
                "Use unavailable+missing_required_input with canonical error.details.questions[] + error.details.recovery_exhaustion when required inputs remain missing after recovery."
            ]
        },
        "session_id": request.session.session_id,
        "snapshot_hash": request.session.snapshot_hash,
        "cursor": request.session.cursor,
        "state_summary": prompt_state_summary,
    });
    if let Some(patch) = patch {
        merge_json_patch(&mut payload, patch);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn render_grounding_prompt_with_patch(
    request: &IntentGroundingRequest,
    patch: Option<&Value>,
) -> String {
    let prompt_state_summary = state_summary_for_prompt(request.state_summary.as_ref());
    let mut payload = json!({
        "schema": "ais-agent-intent/0.0.1",
        "intent": request.intent,
        "tool": "plan.ground_intent",
        "grounding_contract": {
            "status_enum": ["proposed", "unavailable", "invalid"],
            "proposed_required_fields": ["ready_for_todos"],
            "recommended_outputs": ["resolved_inputs", "intent_facts", "confidence", "questions"],
            "confidence_scale": "0-100",
            "rules": [
                "Extract deterministic initial inputs/facts for downstream planning.",
                "Use high confidence only for direct grounding into resolved_inputs.",
                "For low-confidence or conflicting fields, provide questions and set ready_for_todos=false.",
                "When status=proposed and ready_for_todos=false, output must include non-empty questions or missing_refs.",
                "When required data is missing after recovery exhaustion, use unavailable + missing_required_input + canonical error.details.questions[] + error.details.recovery_exhaustion."
            ],
            "actionability_examples": {
                "good": [
                    {
                        "status":"proposed",
                        "ready_for_todos":false,
                        "questions":[{"id":"inputs.owner","question":"What owner address should be used?"}]
                    },
                    {
                        "status":"proposed",
                        "ready_for_todos":false,
                        "missing_refs":["inputs.token.decimals"]
                    }
                ],
                "bad": [
                    {
                        "status":"proposed",
                        "ready_for_todos":false
                    },
                    {
                        "status":"proposed",
                        "ready_for_todos":false,
                        "questions":[],
                        "missing_refs":[]
                    }
                ]
            }
        },
        "session_id": request.session.session_id,
        "snapshot_hash": request.session.snapshot_hash,
        "cursor": request.session.cursor,
        "state_summary": prompt_state_summary,
    });
    if let Some(patch) = patch {
        merge_json_patch(&mut payload, patch);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
fn render_segment_prompt(tool: &str, request: &SegmentPlanningRequest) -> String {
    render_segment_prompt_with_patch(tool, request, None)
}

fn render_segment_prompt_with_patch(
    tool: &str,
    request: &SegmentPlanningRequest,
    patch: Option<&Value>,
) -> String {
    let prompt_state_summary = state_summary_for_prompt(request.state_summary.as_ref());
    let mut payload = json!({
        "schema": "ais-agent-intent/0.0.1",
        "intent": request.intent,
        "tool": tool,
        "repair_instructions": {
            "when": "If tool=plan.revise_segment and previous_error.phase=planning (planner_invalid_tool_output)",
            "goal": "Fix the finalize tool output shape only; do not change plan semantics.",
            "order": ["shape", "ref", "slot", "semantic"],
            "rules": [
                "Return exactly one finalize tool call matching the tool.",
                "Use status=proposed and include a valid segment object; stringified JSON object text is tolerated but object form is preferred.",
                "Keep segment_id/cursor_in/cursor_out and steps as close as possible to your last attempt; only fix missing/wrong fields and types.",
                "If previous_error.last_failed_finalize exists, treat it as baseline draft and patch minimally to satisfy schema/refs/slots.",
                "Fix unknown_input_ref and missing_required_input slot wiring before semantic rewrites.",
                "For unknown_input_ref repair, token/address params should map to address-like refs (for example *.address); *.decimals refs cannot substitute token/address slots.",
                "Never output legacy branch-tree keys (if_true/if_false/then/else/children); branch is encoded by normal flat steps + when/depends_on."
            ]
        },
        "segment_contract": {
            "required_step_fields": ["id", "kind", "inputs"],
            "candidate_ref_rule": "required for query/action; optional for assert/branch control steps",
            "kind_enum": ["action", "query", "assert", "branch"],
            "optional_runtime_controls": ["until", "retry", "timeout_ms"],
            "forbidden_step_fields": ["if_true", "if_false", "then", "else", "children", "steps_if_true", "steps_if_false"],
            "notes": "depends_on references step ids inside the same segment",
            "branch_encoding": "use flat steps; express branch path by when.cel and dependencies. Do not nest child steps under a branch step."
        },
        "value_ref_contract": {
            "allowed": ["lit", "ref", "cel", "object", "array"],
            "examples": [
                {"amount": {"lit": "10"}},
                {"owner": {"ref": "inputs.owner"}},
                {"ok": {"cel": "nodes.q_balance.outputs.balance > 100"}}
            ],
            "cel_namespaces": ["inputs", "params", "nodes", "query", "calculated", "policy", "ctx", "contracts"],
            "node_ref_rule": "Use nodes.<step_id>.outputs.<field> and same-segment step ids only; do not use segment/step path."
        },
        "asset_param_contract": {
            "rule": "for param type=asset, input must resolve to object with address",
            "preferred_shape": {"object":{"address":{"lit":"0x..."}, "chain_ref":{"lit":"eip155:..."}}},
            "shorthand": ["token: \"0x...\"", "token: {\"lit\":\"0x...\"}", "token: {\"object\":{\"address\":{\"lit\":\"0x...\"},\"chain_id\":{\"lit\":\"eip155:...\"}}}"],
            "note": "compiler normalizes chain_ref to chain_id and may normalize shorthand to preferred asset object"
        },
        "write_gate_contract": {
            "scope": "transfer/swap-like action steps",
            "required_pattern": "query -> assert|branch -> action",
            "requirements": [
                "action.depends_on must include at least one assert|branch gate step",
                "gate(assert|branch).depends_on must include query step ids in the same segment (directly or via gate->gate chain)",
                "gate step must be backed by query facts in the same segment",
                "if facts are missing, call catalog.resolve_missing_facts with missing_refs and add matched query steps (e.g. decimals query) before write; if none available return missing_required_input"
            ],
            "minimal_template": {
                "query_step": {"id":"q_balance","kind":"query","candidate_ref":"...","inputs":{}},
                "gate_step": {"id":"g_balance_ok","kind":"assert","depends_on":["q_balance"],"inputs":{},"when":{"cel":"nodes.q_balance.outputs.balance > inputs.threshold"}},
                "action_step": {"id":"a_transfer","kind":"action","candidate_ref":"...","depends_on":["g_balance_ok"],"inputs":{}}
            }
        },
        "schema_lookup_contract": {
            "rule": "If you are unsure about schema fields or CEL/ValueRef usage, call guide.get before finalizing.",
            "examples": [
                {"schema":"ais-plan-sketch/0.1.0"},
                {"schema":"ais-agent-intent/0.0.1"},
                {"topic":"cel"},
                {"topic":"valueref"}
            ],
            "typing_examples": {
                "good": [
                    {"schema":"ais-plan-sketch/0.1.0"},
                    {"schema":"ais-plan-sketch/0.1.0","full":true},
                    {"topic":"cel"}
                ],
                "bad": [
                    {"schema":{"id":"ais-plan-sketch/0.1.0"}},
                    {"topic":{"name":"cel"}},
                    {"schema":"ais-plan-sketch/0.1.0","full":"true"}
                ]
            }
        },
        "tool_call_typing_contract": {
            "rule": "All tool arguments/finalize payloads must use strict JSON schema types; do not quote booleans or numbers.",
            "examples": {
                "good": [{"full": true}, {"done": false}, {"limit": 5}, {"cursor": "0"}],
                "bad": [{"full": "true"}, {"done": "false"}, {"limit": "5"}]
            }
        },
        "self_check_before_tool_or_finalize": {
            "checklist": [
                "Tool is allowed in current phase and finalize tool (if any) is last.",
                "Arguments include required fields and avoid unsupported keys.",
                "JSON types exactly match schema (bool/number are not quoted strings).",
                "guide.get uses canonical single-kind shape: either {schema:\"...\"} or {topic:\"...\"}.",
                "For schema lookups, use full:true only when digest is insufficient."
            ]
        },
        "check_segment_contract": {
            "rule": "Before finalizing proposed/revised segment, you must call plan.check_segment and only finalize when result.ok=true.",
            "segment_binding_rule": "If you change the segment after a successful check, you must run plan.check_segment again for the updated segment."
        },
        "depends_on_contract": {
            "rule": "depends_on items must reference known step ids in the same segment",
            "examples": ["q_native_balance", "q_token_balance"]
        },
        "input_ref_semantic_contract": {
            "rule": "For unknown_input_ref repair, preserve slot semantics: token/address params map to address-like refs (for example *.address); *.decimals refs are only for decimal slots.",
            "negative_examples": [
                {
                    "param": "token",
                    "expected_ref_like": "*.address",
                    "invalid_ref": "inputs.token.decimals"
                }
            ]
        },
        "failure_contract": {
            "unavailable_or_invalid": {
                "required_fields": ["status", "done", "error.reason_code"],
                "status_enum": ["unavailable", "invalid"]
            },
            "missing_required_input": {
                "when": "status=unavailable and error.reason_code=missing_required_input",
                "required_fields": ["error.details.questions", "error.details.recovery_exhaustion.unresolved_refs", "error.details.recovery_exhaustion.reasons", "error.details.recovery_exhaustion.attempt_trace_id"],
                "question_shape": {
                    "id": "string",
                    "question": "string",
                    "required": "boolean(optional)",
                    "options": [
                        {
                            "label": "string",
                            "description": "string(optional)",
                            "value": "any(optional)"
                        }
                    ]
                },
                "recovery_exhaustion_shape": {
                    "unresolved_refs": ["inputs.<slot>"],
                    "reasons": ["string(non-empty)"],
                    "attempt_trace_id": "string(non-empty)"
                }
            }
        },
        "todo_contract": {
            "rule": "Host enforces one todo per segment (1 todo = 1 segment).",
            "current_todo_path": "state_summary.todo_state.current_todo",
            "requirements": [
                "Produce exactly one segment that advances the current todo only.",
                "Do not combine unrelated objectives in a single segment."
            ]
        },
        "session_id": request.session.session_id,
        "snapshot_hash": request.session.snapshot_hash,
        "cursor": request.session.cursor,
        "state_summary": prompt_state_summary,
        "previous_error": request.previous_error,
        "last_segment": request.last_segment,
    });
    if let Some(patch) = patch {
        merge_json_patch(&mut payload, patch);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn state_summary_for_prompt(state_summary: Option<&Value>) -> Value {
    state_summary
        .and_then(|summary| summary.pointer("/prompt_compact").cloned())
        .or_else(|| state_summary.cloned())
        .unwrap_or(Value::Null)
}

fn compact_failed_finalize_payload(
    call: &ToolCall,
    assistant_content: Option<&str>,
    round: u8,
) -> Value {
    let raw = json!({
        "round": round,
        "tool_call_id": call.id,
        "tool": call.name,
        "arguments": call.arguments,
        "assistant_content": assistant_content,
    });
    let sanitized = sanitize_for_llm_payload(&raw);
    compact_json_with_options(
        &sanitized,
        &JsonBudgetOptions {
            max_depth: 8,
            max_object_entries: 96,
            max_array_items: 48,
            max_string_chars: 1600,
        },
    )
}

fn merge_json_patch(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_obj), Value::Object(patch_obj)) => {
            for (key, patch_value) in patch_obj {
                if let Some(base_value) = base_obj.get_mut(key) {
                    merge_json_patch(base_value, patch_value);
                } else {
                    base_obj.insert(key.clone(), patch_value.clone());
                }
            }
        }
        (base_slot, patch_value) => {
            *base_slot = patch_value.clone();
        }
    }
}

pub(super) fn candidate_snapshot(
    candidate_context: Option<&CandidateContext>,
    filter: Option<ListCandidatesFilterArgs>,
) -> Value {
    candidate_context
        .map(|context| {
            let grouped = grouped_candidate_snapshot(context, filter.as_ref());
            let sanitized = sanitize_for_llm_payload(&grouped);
            compact_json_with_options(
                &sanitized,
                &JsonBudgetOptions {
                    max_depth: 8,
                    ..JsonBudgetOptions::default()
                },
            )
        })
        .unwrap_or_else(|| {
            json!({
                "schema":"ais-executable-candidates/0.0.1",
                "unavailable": true,
                "reason": "workspace candidate context unavailable"
            })
        })
}

fn grouped_candidate_snapshot(
    context: &CandidateContext,
    filter: Option<&ListCandidatesFilterArgs>,
) -> Value {
    let actions = context
        .index_candidates
        .get("actions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let queries = context
        .index_candidates
        .get("queries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let execution_plugins = context
        .index_candidates
        .get("execution_plugins")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));

    let mut ref_chains = BTreeMap::<String, BTreeSet<String>>::new();
    for card in context
        .executable_candidates
        .actions
        .iter()
        .chain(context.executable_candidates.queries.iter())
    {
        let Some(reference) = card.get("ref").and_then(Value::as_str) else {
            continue;
        };
        let chains = card
            .get("execution_chains")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        ref_chains
            .entry(reference.to_string())
            .or_default()
            .extend(chains);
    }
    let mut ref_required_inputs = BTreeMap::<String, Vec<String>>::new();
    for (reference, detail) in &context.detail_by_ref {
        let required_inputs = detail
            .get("params")
            .and_then(Value::as_array)
            .map(|params| {
                let mut names = params
                    .iter()
                    .filter_map(|param| {
                        let required = param
                            .get("required")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let name = param.get("name").and_then(Value::as_str)?;
                        required.then_some(name.to_string())
                    })
                    .collect::<Vec<_>>();
                names.sort();
                names.dedup();
                names
            })
            .unwrap_or_default();
        ref_required_inputs.insert(reference.clone(), required_inputs);
    }

    let mut grouped = BTreeMap::<String, Value>::new();
    let mut append = |kind: &str, card: &Value| {
        let Some(reference) = card.get("ref").and_then(Value::as_str) else {
            return;
        };
        let protocol = card
            .get("schema_name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| reference.split('/').next().unwrap_or_default())
            .to_string();
        if let Some(protocol_filter) = filter.and_then(|item| item.protocol.as_ref()) {
            if !protocol
                .to_ascii_lowercase()
                .contains(protocol_filter.as_str())
            {
                return;
            }
        }
        let chains = ref_chains
            .get(reference)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(chain_filter) = filter.and_then(|item| item.chain.as_ref()) {
            let matches_chain = chains
                .iter()
                .any(|chain| list_chain_filter_matches(chain_filter, chain));
            if !matches_chain {
                return;
            }
        }

        let entry = grouped.entry(protocol.clone()).or_insert_with(|| {
            json!({
                "protocol": protocol,
                "chains": [],
                "actions": [],
                "queries": []
            })
        });
        if let Some(chains_value) = entry.get_mut("chains").and_then(Value::as_array_mut) {
            for chain in &chains {
                if !chains_value
                    .iter()
                    .any(|item| item == &Value::String(chain.clone()))
                {
                    chains_value.push(Value::String(chain.clone()));
                }
            }
        }
        let target = if kind == "action" {
            entry.get_mut("actions")
        } else {
            entry.get_mut("queries")
        };
        if let Some(items) = target.and_then(Value::as_array_mut) {
            let required_inputs = ref_required_inputs
                .get(reference)
                .cloned()
                .unwrap_or_default();
            items.push(json!({
                "ref": reference,
                "chains": chains,
                "required_inputs": required_inputs
            }));
        }
    };

    for action in &actions {
        append("action", action);
    }
    for query in &queries {
        append("query", query);
    }

    json!({
        "schema": context
            .index_candidates
            .get("schema")
            .cloned()
            .unwrap_or_else(|| Value::String("ais-executable-candidates/0.0.1".to_string())),
        "level": "name_only_grouped",
        "hash": context.index_candidates.get("hash").cloned(),
        "catalog_schema": context.index_candidates.get("catalog_schema").cloned(),
        "catalog_hash": context.index_candidates.get("catalog_hash").cloned(),
        "filters": {
            "chain": filter.and_then(|item| item.chain.clone()),
            "protocol": filter.and_then(|item| item.protocol.clone()),
        },
        "protocols": grouped.into_values().collect::<Vec<_>>(),
        "execution_plugins": execution_plugins,
    })
}

fn list_chain_filter_matches(filter: &str, candidate_chain: &str) -> bool {
    let normalized_filter = filter.trim().to_ascii_lowercase();
    let normalized_candidate = candidate_chain.trim().to_ascii_lowercase();
    if normalized_filter.is_empty() {
        return true;
    }
    if normalized_filter == "*" || normalized_candidate == "*" {
        return true;
    }
    if normalized_filter == normalized_candidate {
        return true;
    }
    if let Some(prefix) = normalized_filter.strip_suffix('*') {
        return normalized_candidate.starts_with(prefix);
    }
    if let Some(prefix) = normalized_candidate.strip_suffix('*') {
        return normalized_filter.starts_with(prefix);
    }
    false
}

pub(super) fn is_control_semantics_query(query: Option<&str>) -> bool {
    let Some(raw_query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let normalized = raw_query.to_ascii_lowercase().replace('-', "_");
    let tokens = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let control_terms = [
        "assert",
        "branch",
        "until",
        "retry",
        "timeout",
        "timeout_ms",
        "depends_on",
        "cel",
        "valueref",
        "value_ref",
    ];
    control_terms
        .iter()
        .any(|term| tokens.iter().any(|token| token == term))
}

pub(super) fn control_semantics_search_hint_payload(
    query: Option<String>,
    kind: Option<String>,
    chain: Option<String>,
    min_risk_level: Option<u8>,
    max_risk_level: Option<u8>,
    limit: Option<usize>,
) -> Value {
    let kind = kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("any")
        .to_ascii_lowercase();
    let limit = limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    json!({
        "schema": "ais-catalog-search-response/0.0.1",
        "level": "name_only",
        "query": query,
        "filters": {
            "kind": kind,
            "chain": chain,
            "min_risk_level": min_risk_level,
            "max_risk_level": max_risk_level
        },
        "limit": limit,
        "total_matches": 0,
        "returned_matches": 0,
        "truncated": false,
        "results": [],
        "hint": {
            "reason_code": "control_semantics_not_catalog_candidate",
            "message": "assert/branch/until/retry are PlanSketch control-step semantics, not protocol action/query candidates.",
            "next_tool": "guide.get",
            "guide_requests": [
                { "schema": "ais-plan-sketch/0.1.0" },
                { "topic": "cel" }
            ]
        }
    })
}

fn extract_round_context_signal(user_prompt: &str) -> RoundContextSignal {
    let parsed = serde_json::from_str::<Value>(user_prompt).ok();
    let pressure_mode = parsed
        .as_ref()
        .and_then(|value| value.pointer("/state_summary/context_budget/pressure_mode"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let compressed = parsed
        .as_ref()
        .map(|value| {
            let diagnostics = value.pointer("/state_summary/context_budget/pack_diagnostics");
            let compressed_total = diagnostics
                .and_then(|item| item.get("compressed_blocks_total"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let evicted_total = diagnostics
                .and_then(|item| item.get("packed_blocks_evicted"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let trace_non_empty = value
                .pointer("/state_summary/context_budget/pack_trace")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty());
            let overflowed = value
                .pointer("/state_summary/context_budget/pack_overflow_reason")
                .and_then(Value::as_str)
                .is_some();
            compressed_total > 0 || evicted_total > 0 || trace_non_empty || overflowed
        })
        .unwrap_or(false);
    let adjudicate_mode = parsed
        .as_ref()
        .and_then(|value| value.pointer("/previous_error/autofill/mode"))
        .and_then(Value::as_str)
        == Some("host_binding_adjudicate_round");
    RoundContextSignal {
        pressure_mode,
        compressed,
        adjudicate_mode,
    }
}

fn catalog_search_result_is_empty(content: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        return false;
    };
    let total = parsed
        .get("total_matches")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let returned = parsed
        .get("returned_matches")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    total == 0 && returned == 0
}

fn catalog_search_signature_from_result(content: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(content).ok()?;
    let query = parsed
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");
    let kind = parsed
        .pointer("/filters/kind")
        .and_then(Value::as_str)
        .unwrap_or("any");
    let chain = parsed
        .pointer("/filters/chain")
        .and_then(Value::as_str)
        .unwrap_or("*");
    Some(format!("{kind}|{chain}|{query}"))
}

fn catalog_search_loop_guard_hint_payload(streak: u64) -> Value {
    json!({
        "loop_guard": {
            "kind": "catalog_search_empty_streak",
            "streak": streak,
            "rule": "Avoid repeating semantically similar catalog.search queries that keep returning empty.",
            "next_step_order": [
                "reuse state_summary.tool_memory_projection.recent.list_inventory if present",
                "if discovery baseline is missing, call list_candidates once",
                "narrow by explicit refs and call get_candidate_detail",
                "for control semantics (assert/branch/until/retry), call guide.get with {\"schema\":\"ais-plan-sketch/0.1.0\"} or {\"topic\":\"cel\"}"
            ]
        }
    })
}

fn adjudicate_finalize_guard_payload(finalize_tool: &str, round: u8, reason: &str) -> Value {
    json!({
        "loop_guard": {
            "kind": "adjudicate_budget_guard",
            "reason": reason,
            "round": round,
            "max_rounds": ADJUDICATE_MAX_TOOL_ROUNDS,
            "required_action": "Finalize now.",
            "contract": format!("Call `{finalize_tool}` exactly once as the last tool call in this response."),
            "rules": [
                "Do not issue more discovery tools in this round.",
                "Return best-effort binding_decisions/query_decisions from available evidence.",
                "If still unresolved, finalize with status=unavailable reason_code=missing_required_input and include error.details.questions[] + error.details.recovery_exhaustion{unresolved_refs[],reasons[],attempt_trace_id}."
            ]
        }
    })
}

fn no_toolcall_repair_payload(
    phase: PlannerRoundPhase,
    finalize_tool: &str,
    round: u8,
    retry_attempt: u8,
    max_retries: u8,
    tools: &[ToolSpec],
) -> Value {
    let allowed_tools = tools
        .iter()
        .map(|tool| Value::String(tool.name.clone()))
        .collect::<Vec<_>>();
    json!({
        "error": {
            "reason_code": "no_tool_calls",
            "message": "No tool calls were returned. You must return at least one allowed tool call.",
            "phase": phase_name(phase),
            "finalize_tool": finalize_tool,
            "round": round,
            "retry_attempt": retry_attempt,
            "max_retries": max_retries,
            "allowed_tools": allowed_tools,
        },
        "rules": [
            "Return at least one tool call in this response.",
            "Use only allowed tools for this phase.",
            "If finishing this phase, call finalize_tool exactly once and as the last tool call."
        ]
    })
}

fn llm_request_to_value(request: &CompleteWithToolsRequest) -> Value {
    json!({
        "messages": request
            .messages
            .iter()
            .map(llm_message_to_value)
            .collect::<Vec<_>>(),
        "tools": request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn llm_response_to_value(response: &ais_llm::CompleteWithToolsResponse) -> Value {
    json!({
        "assistant_content": response.assistant_content,
        "tool_calls": response
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "name": call.name,
                    "arguments": call.arguments,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn llm_message_to_value(message: &LlmMessage) -> Value {
    json!({
        "role": format!("{:?}", message.role).to_ascii_lowercase(),
        "content": message.content,
        "tool_name": message.tool_name,
        "tool_call_id": message.tool_call_id,
        "tool_calls": message
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id,
                    "name": call.name,
                    "arguments": call.arguments,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn control_step_candidate_ref_hint_payload() -> Value {
    json!({
        "repair_hint": {
            "kind": "control_step_candidate_ref",
            "reason_code": "control_step_candidate_not_found",
            "rule": "Do not search catalog for synthetic refs like `.../assert` or `.../branch`. Control steps are built in.",
            "fix": [
                "For kind=assert|branch, keep kind as assert/branch and use when.cel / inputs.condition to express gate logic.",
                "Then express gate semantics via when.cel and depends_on (query -> assert|branch -> action).",
                "Re-run plan.check_segment and finalize only when ok=true."
            ]
        }
    })
}

fn summarize_tool_message(tool_name: &str, content: &str) -> String {
    let parsed = serde_json::from_str::<Value>(content).ok();
    match (tool_name, parsed) {
        ("list_candidates", Some(value)) => {
            let protocols = value
                .get("protocols")
                .and_then(Value::as_array)
                .map(|arr| arr.len())
                .unwrap_or(0);
            let action_count = value
                .get("protocols")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .map(|item| {
                            item.get("actions")
                                .and_then(Value::as_array)
                                .map(|items| items.len())
                                .unwrap_or(0)
                        })
                        .sum::<usize>()
                })
                .unwrap_or(0);
            let query_count = value
                .get("protocols")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .map(|item| {
                            item.get("queries")
                                .and_then(Value::as_array)
                                .map(|items| items.len())
                                .unwrap_or(0)
                        })
                        .sum::<usize>()
                })
                .unwrap_or(0);
            format!("summary(protocols={protocols},actions={action_count},queries={query_count})")
        }
        ("get_candidate_detail", Some(value)) => {
            let items = value
                .get("details")
                .and_then(Value::as_array)
                .map(|arr| arr.len())
                .unwrap_or_else(|| value.as_array().map(|arr| arr.len()).unwrap_or(0));
            format!("summary(items={items})")
        }
        ("catalog.search", Some(value)) => {
            let total = value
                .get("total_matches")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let returned = value
                .get("returned_matches")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let hint = value
                .pointer("/hint/reason_code")
                .and_then(Value::as_str)
                .unwrap_or("-");
            if hint == "-" {
                format!("summary(total={total},returned={returned})")
            } else {
                format!("summary(total={total},returned={returned},hint={hint})")
            }
        }
        ("catalog.resolve_missing_facts", Some(value)) => {
            let resolved = value
                .get("resolved")
                .and_then(Value::as_array)
                .map(|items| items.len())
                .unwrap_or(0);
            let unresolved = value
                .get("unresolved_refs")
                .and_then(Value::as_array)
                .map(|items| items.len())
                .unwrap_or(0);
            let candidates = value
                .get("resolved")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            item.get("query_candidates")
                                .and_then(Value::as_array)
                                .map(|candidates| candidates.len())
                                .unwrap_or(0)
                        })
                        .sum::<usize>()
                })
                .unwrap_or(0);
            format!("summary(resolved={resolved},unresolved={unresolved},candidates={candidates})")
        }
        ("guide.get", Some(value)) => {
            let kind = value.get("kind").and_then(Value::as_str).unwrap_or("-");
            let has_error = value.get("error").is_some();
            let schema_id = value
                .pointer("/schema/id")
                .and_then(Value::as_str)
                .unwrap_or("-");
            let topic = value
                .pointer("/topic/topic")
                .and_then(Value::as_str)
                .unwrap_or("-");
            format!("summary(kind={kind},error={has_error},schema_id={schema_id},topic={topic})")
        }
        ("plan.check_segment", Some(value)) => {
            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let issues = value
                .get("issues")
                .and_then(Value::as_array)
                .map(|items| items.len())
                .unwrap_or(0);
            let segment_id = value
                .get("segment_id")
                .and_then(Value::as_str)
                .unwrap_or("-");
            format!("summary(ok={ok},segment_id={segment_id},issues={issues})")
        }
        _ => format!("summary(raw={})", truncate_for_log(content, 240)),
    }
}

fn truncate_for_log(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }
    let clipped = input.chars().take(max_len).collect::<String>();
    format!("{clipped}...")
}

fn estimate_tokens_from_json<T: Serialize>(value: &T) -> u64 {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    let chars = encoded.chars().count();
    chars
        .saturating_add(LLM_CHARS_PER_TOKEN_ESTIMATE.saturating_sub(1))
        .checked_div(LLM_CHARS_PER_TOKEN_ESTIMATE)
        .unwrap_or(0) as u64
}

#[cfg(test)]
#[path = "tests/intent_segmented_module.rs"]
mod tests;
