use crate::error::RunnerError;
use ais_core::{stable_hash_hex, StableJsonOptions};
use ais_llm::{CompleteWithToolsRequest, LlmMessage, LlmProvider, MessageRole, ToolCall, ToolSpec};
use ais_schema::{
    get_json_schema,
    versions::{SCHEMA_AGENT_PLANNING_TOOLS_0_1_0, SCHEMA_PLAN_SKETCH_0_1_0},
};
use ais_sdk::documents::PlanSketchSegment;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::budget::{compact_json_for_llm, compact_json_with_options, JsonBudgetOptions};
use super::candidates::{
    CandidateContext, CandidateSearchRequest, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT,
};
use super::planning_memory::{PlanningMemory, PlanningMemoryBudget};
use super::sanitize::{sanitize_for_llm_payload, sanitize_for_llm_payload_with_limit};
use super::todos::TodoSpec;

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
    last_failed_finalize: Option<Value>,
    begin_context: Option<PlannerBeginContext>,
    prompt_builder: SegmentedPromptContextBuilder,
    prompt_overrides: SegmentedPromptOverrides,
    max_tool_rounds: u8,
    verbose_llm: bool,
    usage_tracker: PlannerLlmUsageTracker,
}

#[derive(Debug, Clone)]
struct PlannerBeginContext {
    pack_snapshot_hash: String,
    chain_scope: Vec<String>,
}

#[derive(Debug, Clone)]
struct SegmentCheckContext {
    intent: String,
    session_id: String,
    cursor: String,
    pack_snapshot_hash: String,
    chain_scope: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannerRoundPhase {
    Begin,
    GroundIntent,
    ProposeTodos,
    ProposeSegment,
    ReviseSegment,
}

const SEGMENTED_PROMPT_VERSION: &str = "aisrs-segmented-planner-v2";
pub(crate) const DEFAULT_SEGMENTED_MAX_TOOL_ROUNDS: u8 = 24;
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
                "Check state_summary.tool_memory_projection first; call discovery/schema tools only when required information is missing or stale.",
                "For schema/topic contracts, call guide.get first using canonical request shape; schema lookups are digest-first and should request {full:true} only when digest is insufficient.",
                "For capability narrowing, prefer catalog.search first (compact ref-first cards: ref/kind/chains?/risk_level?), then get_candidate_detail for selected refs.",
                "Use list_candidates as broad inventory only when needed, and avoid repeating it in the same snapshot scope.",
                "assert/branch/until/retry are PlanSketch control-step semantics, not catalog candidates.",
                "Control-step semantics are not catalog candidates, but every step (including assert/branch) still requires candidate_ref from discovered candidates.",
                "Never use catalog.search to look up control-step semantics; use guide.get ({schema:\"ais-plan-sketch/0.1.0\"} / {topic:\"cel\"}).",
                "Tool results for list_candidates/catalog.search/get_candidate_detail are cached by snapshot scope; repeated identical calls return cached snapshots.",
                "A segment is PlanSketch-compatible: segment_id/cursor_in/cursor_out/done/steps.",
                "Use state_summary.todo_state.current_todo as the only objective for this round; output exactly one segment for that todo.",
                "Use only refs listed in state_summary.input_registry.known_refs for inputs.* bindings; do not invent unknown input refs.",
                "Each step needs id + kind + candidate_ref + inputs; depends_on references step ids in the same segment.",
                "In CEL/ValueRef node refs, use nodes.<step_id>.outputs.<field> with same-segment step ids only (no segment/step path).",
                "Use only refs from candidates; never invent protocols/actions.",
                "For errors, return status=invalid|unavailable with error.reason_code; for missing_required_input include error.details.questions[].",
                "Repair order is strict: shape -> ref -> slot -> semantic. Do not rewrite semantics before shape/ref/slot are valid.",
                "Keep segments small and deterministic; prefer read segment before write segment.",
                "For transfer/swap writes, include pre-write gate chain query -> assert|branch -> action in the same segment.",
                "Follow the phase-specific tool policy exactly.",
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
                "Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, and one final plan.ground_intent (must be last).",
                "If schema/topic contract is needed, call guide.get before discovery tools.",
                "For candidate narrowing, use catalog.search (ref-first compact cards) then get_candidate_detail for selected refs.",
                "Goal: extract deterministic initial inputs/facts from intent before todo planning.",
                "Return status=proposed with ready_for_todos=true only when key fields are high-confidence.",
                "Low-confidence or conflicting fields must be returned via missing_required_input questions (not guessed).",
                "Do not call plan.begin, plan.propose_todos, plan.propose_segment, or plan.revise_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_todos: vec![
                "Current phase: propose_todos.",
                "Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, and one final plan.propose_todos (must be last).",
                "If schema/topic contract is needed, call guide.get before discovery tools.",
                "For capability narrowing, use catalog.search (ref-first compact cards) then get_candidate_detail for selected refs.",
                "assert/branch/until/retry semantics are control-step rules, not catalog candidates; use guide.get for these semantics.",
                "Output deterministic todos for the whole intent before segment planning.",
                "Each todo must include title; optional fields: required_facts/produced_facts/acceptance.",
                "Prefer 2-4 concise todos; avoid duplicates or overlapping objectives.",
                "Do not call plan.begin, plan.propose_segment, or plan.revise_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_propose: vec![
                "Current phase: propose_segment.",
                "Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, plan.check_segment, and one final plan.propose_segment (must be last).",
                "Host enforces 1 todo = 1 segment; plan only for current state_summary.todo_state.current_todo.",
                "If schema/topic contract is needed, call guide.get first; for capability narrowing, use catalog.search (ref-first compact cards) then get_candidate_detail.",
                "assert/branch/until/retry semantics must be read from guide.get, not catalog.search.",
                "Even for assert/branch control steps, candidate_ref is required in steps[] and must be a discovered candidate ref.",
                "If uncertain about output shape or CEL/ValueRef contracts, call guide.get using canonical shape {schema:\"...\"} or {topic:\"...\"} before finalizing.",
                "You must call plan.check_segment and only finalize when check result has ok=true.",
                "Repair order is strict: shape -> ref -> slot -> semantic.",
                "Transfer/swap actions must depend on an assert/branch gate step backed by query facts in the same segment.",
                "If token decimals are unknown, add a decimals query (erc20/decimals or equivalent) before write; otherwise return missing_required_input.",
                "For volatile facts (balance/allowance), include a fresh query in the same segment before write; do not rely on stale context-only values.",
                "If compile errors mention unknown_input_ref, fix refs to entries from state_summary.input_registry.known_refs first.",
                "If required facts are missing, return unavailable with reason_code=missing_required_input and include error.details.questions[].",
                "Never call plan.begin or plan.revise_segment.",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            phase_rules_revise: vec![
                "Current phase: revise_segment.",
                "Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, plan.check_segment, and one final plan.revise_segment (must be last).",
                "Keep repairing the same current todo from state_summary.todo_state.current_todo; do not switch to a different objective.",
                "If schema/topic contract is needed, call guide.get first; for capability narrowing, use catalog.search (ref-first compact cards) then get_candidate_detail.",
                "assert/branch/until/retry semantics must be read from guide.get, not catalog.search.",
                "Even for assert/branch control steps, candidate_ref is required in steps[] and must be a discovered candidate ref.",
                "Use guide.get for contract lookups with canonical shape {schema:\"...\"} or {topic:\"...\"} instead of guessing field names.",
                "You must call plan.check_segment and only finalize when check result has ok=true.",
                "Repair order is strict: shape -> ref -> slot -> semantic; keep semantic edits minimal.",
                "Maintain transfer/swap pre-write gates (query -> assert|branch -> action) while repairing.",
                "If decimals/facts are missing for token writes, prefer adding query steps or return missing_required_input with questions.",
                "When repairing write paths, ensure volatile facts (balance/allowance) are refreshed by same-segment query steps.",
                "If compile errors mention unknown_input_ref, replace guessed refs using state_summary.input_registry.known_refs.",
                "If required facts are missing, return unavailable with reason_code=missing_required_input and include error.details.questions[].",
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
        let workspace_summary = workspace_summary(candidate_context);
        let pack_summary = "Pack summary source: request.pack_snapshot_hash + request.chain_scope.";
        let modules = json!({
            "version": SEGMENTED_PROMPT_VERSION,
            "phase": phase_name(phase),
            "base_rules": self.base_rules,
            "phase_rules": phase_rules,
            "contracts_summary": self.contracts_summary,
            "pack_summary": pack_summary,
            "workspace_summary": workspace_summary,
        });
        let hash = stable_hash_hex(&modules, &StableJsonOptions::default())
            .unwrap_or_else(|_| "prompt-hash-unavailable".to_string());
        let prompt = format!(
            "You are an AIS segmented planner.\nPrompt-Version: {SEGMENTED_PROMPT_VERSION}\nPrompt-Hash: {hash}\n\nBase Rules:\n{}\n\nPhase Rules:\n{}\n\nContracts Summary:\n{}\n\nPack Summary:\n- {pack_summary}\n\nWorkspace Summary:\n{}",
            numbered_lines(self.base_rules.as_slice()),
            numbered_lines(phase_rules.as_slice()),
            numbered_lines(self.contracts_summary.as_slice()),
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
            last_failed_finalize: None,
            begin_context: None,
            prompt_builder: SegmentedPromptContextBuilder::default(),
            prompt_overrides: SegmentedPromptOverrides::default(),
            max_tool_rounds: DEFAULT_SEGMENTED_MAX_TOOL_ROUNDS,
            verbose_llm: false,
            usage_tracker: PlannerLlmUsageTracker::default(),
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
        self.planning_memory
            .restore_from_checkpoint(value, PlanningMemoryBudget::default())
    }

    pub fn planning_memory_checkpoint_value(&self) -> Option<Value> {
        self.planning_memory
            .checkpoint_value(PlanningMemoryBudget::default())
    }

    pub fn llm_usage_value(&self) -> Value {
        self.usage_tracker.to_value()
    }

    pub fn tool_memory_projection_value(&self, max_tokens: usize) -> Option<Value> {
        self.planning_memory.tool_memory_projection(max_tokens)
    }

    pub fn take_last_failed_finalize(&mut self) -> Option<Value> {
        self.last_failed_finalize.take()
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

        for round in 0..self.max_tool_rounds {
            let llm_request = CompleteWithToolsRequest {
                messages: messages.clone(),
                tools: tools.clone(),
            };
            let response = self
                .provider
                .complete_with_tools(llm_request.clone())
                .map_err(|error| RunnerError::Llm(error.to_string()))?;
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
                return Err(RunnerError::Llm(
                    "segmented planner provider returned no tool calls".to_string(),
                ));
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
            for call in &response.tool_calls {
                let decoded = match decode_segmented_tool_call_with_memory(
                    call,
                    finalize_tool,
                    phase,
                    self.candidate_context.as_ref(),
                    segment_check_context,
                    Some(&mut self.planning_memory),
                ) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        if call.name == finalize_tool {
                            self.last_failed_finalize = Some(compact_failed_finalize_payload(
                                call,
                                response.assistant_content.as_deref(),
                                round.saturating_add(1),
                            ));
                        }
                        return Err(error);
                    }
                };
                match decoded {
                    DecodedSegmentedToolCall::Final(result) => {
                        if require_successful_segment_check
                            && !latest_segment_check_ok
                            && finalized_segment_is_proposed(&result)
                        {
                            let payload = missing_pre_finalize_check_payload(finalize_tool);
                            let content =
                                serde_json::to_string(&payload).map_err(RunnerError::from)?;
                            if self.verbose_llm {
                                eprintln!(
                                    "[llm] tool_result tool_call_id={} tool={} cached=false {}",
                                    call.id,
                                    call.name,
                                    summarize_tool_message(call.name.as_str(), content.as_str())
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
                        return Ok(result);
                    }
                    DecodedSegmentedToolCall::ToolMessage {
                        tool_name,
                        tool_call_id,
                        content,
                        cached,
                    } => {
                        if tool_name == "plan.check_segment" {
                            latest_segment_check_ok = plan_check_result_ok(content.as_str());
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
        })
    }
}

#[derive(Debug, Deserialize)]
struct CandidateDetailArgs {
    refs: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CatalogSearchArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    chain: Option<String>,
    #[serde(default)]
    min_risk_level: Option<u8>,
    #[serde(default)]
    max_risk_level: Option<u8>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct GuideGetArgs {
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    full: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CheckSegmentArgs {
    segment: Value,
}

#[derive(Debug, Deserialize)]
struct BeginLimits {
    max_rounds: u8,
    max_segments: u8,
}

#[derive(Debug, Deserialize)]
struct BeginToolArgs {
    session_id: Value,
    snapshot_hash: Value,
    cursor: Value,
    limits: BeginLimits,
}

#[derive(Debug, Deserialize)]
struct SegmentError {
    reason_code: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    details: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SegmentToolArgs {
    status: String,
    done: bool,
    #[serde(default)]
    segment: Option<Value>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    cursor_next: Option<Value>,
    #[serde(default)]
    issues: Vec<Value>,
    #[serde(default)]
    error: Option<SegmentError>,
    #[serde(default)]
    questions: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct TodoToolArgs {
    status: String,
    #[serde(default)]
    todos: Vec<TodoSpec>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    issues: Vec<Value>,
    #[serde(default)]
    error: Option<SegmentError>,
    #[serde(default)]
    questions: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct GroundingToolArgs {
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
    issues: Vec<Value>,
    #[serde(default)]
    error: Option<SegmentError>,
    #[serde(default)]
    questions: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MissingInputOption {
    #[serde(default)]
    value: Option<Value>,
    label: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MissingInputQuestion {
    id: String,
    question: String,
    #[serde(default)]
    options: Vec<MissingInputOption>,
    #[serde(default)]
    required: Option<bool>,
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

fn guide_get_payload(args: GuideGetArgs) -> Value {
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

#[derive(Debug)]
enum PlannerToolOutput {
    Begin(SegmentPlanningSession),
    SegmentDraft(SegmentDraft),
    TodoDraft(TodoDraft),
    IntentGrounding(IntentGroundingDraft),
}

#[derive(Debug)]
enum DecodedSegmentedToolCall {
    Final(PlannerToolOutput),
    ToolMessage {
        tool_name: String,
        tool_call_id: String,
        content: String,
        cached: bool,
    },
}

#[cfg(test)]
fn decode_segmented_tool_call(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    decode_segmented_tool_call_with_memory(
        tool,
        finalize_tool,
        phase,
        candidate_context,
        None,
        None,
    )
}

fn decode_segmented_tool_call_with_memory(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
    segment_check_context: Option<&SegmentCheckContext>,
    memory: Option<&mut PlanningMemory>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    ensure_tool_allowed_for_phase(tool.name.as_str(), phase)?;

    let cache_key = tool_cache_key(tool.name.as_str(), &tool.arguments);
    let require_guide_schema_full =
        guide_get_requires_full_schema(tool.name.as_str(), &tool.arguments);
    if let (Some(memory), Some(cache_key)) = (memory.as_ref(), cache_key.as_ref()) {
        if let Some(content) = memory.get(cache_key.as_str()) {
            let can_use_cached =
                !require_guide_schema_full || guide_get_payload_contains_full_schema(content);
            if can_use_cached {
                return Ok(DecodedSegmentedToolCall::ToolMessage {
                    tool_name: tool.name.clone(),
                    tool_call_id: tool.id.clone(),
                    content: content.to_string(),
                    cached: true,
                });
            }
        }
    }

    match tool.name.as_str() {
        "list_candidates" => {
            let content = serde_json::to_string(&candidate_snapshot(candidate_context))
                .map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "list_candidates".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "get_candidate_detail" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "candidate detail tool is unavailable".to_string(),
                ));
            };
            let args: CandidateDetailArgs = serde_json::from_value(tool.arguments.clone())
                .map_err(|error| {
                    RunnerError::Llm(format!("invalid get_candidate_detail args: {error}"))
                })?;
            let details = context.get_details_for_refs(&args.refs);
            let sanitized = sanitize_for_llm_payload(&details);
            let compacted = compact_json_with_options(
                &sanitized,
                &JsonBudgetOptions {
                    max_depth: 8,
                    ..JsonBudgetOptions::default()
                },
            );
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "get_candidate_detail".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "catalog.search" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "catalog search tool is unavailable".to_string(),
                ));
            };
            let args: CatalogSearchArgs =
                serde_json::from_value(tool.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid catalog.search args: {error}"))
                })?;
            let query = args.query;
            let searched = if is_control_semantics_query(query.as_deref()) {
                control_semantics_search_hint_payload(
                    query.clone(),
                    args.kind.clone(),
                    args.chain.clone(),
                    args.min_risk_level,
                    args.max_risk_level,
                    args.limit,
                )
            } else {
                context.search_candidates(&CandidateSearchRequest {
                    query,
                    kind: args.kind,
                    chain: args.chain,
                    min_risk_level: args.min_risk_level,
                    max_risk_level: args.max_risk_level,
                    limit: args.limit,
                })
            };
            let sanitized = sanitize_for_llm_payload(&searched);
            let compacted = compact_json_for_llm(&sanitized);
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "catalog.search".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "guide.get" => {
            let args: GuideGetArgs = serde_json::from_value(tool.arguments.clone())
                .map_err(|error| RunnerError::Llm(format!("invalid guide.get args: {error}")))?;
            let payload = guide_get_payload(args);
            let is_schema_request = payload.get("kind").and_then(Value::as_str) == Some("schema");
            let schema_has_full_json = payload.pointer("/schema/json").is_some();
            let sanitized = if is_schema_request && schema_has_full_json {
                sanitize_for_llm_payload_with_limit(&payload, 16_000)
            } else {
                sanitize_for_llm_payload(&payload)
            };
            let compacted = if is_schema_request {
                if schema_has_full_json {
                    compact_json_with_options(
                        &sanitized,
                        &JsonBudgetOptions {
                            max_depth: 64,
                            max_object_entries: 4_096,
                            max_array_items: 1_024,
                            max_string_chars: 16_000,
                        },
                    )
                } else {
                    compact_json_with_options(
                        &sanitized,
                        &JsonBudgetOptions {
                            max_depth: 10,
                            max_object_entries: 128,
                            max_array_items: 64,
                            max_string_chars: 1600,
                        },
                    )
                }
            } else {
                compact_json_with_options(
                    &sanitized,
                    &JsonBudgetOptions {
                        max_depth: 8,
                        max_object_entries: 64,
                        max_array_items: 24,
                        max_string_chars: 2400,
                    },
                )
            };
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "guide.get".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "plan.check_segment" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "plan.check_segment requires workspace candidate context".to_string(),
                ));
            };
            let Some(check_context) = segment_check_context else {
                return Err(RunnerError::Llm(
                    "plan.check_segment is unavailable before plan.begin".to_string(),
                ));
            };
            let args: CheckSegmentArgs =
                serde_json::from_value(tool.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid plan.check_segment args: {error}"))
                })?;
            let segment = decode_plan_sketch_segment_arg(&args.segment)?;
            let payload = match super::compile_segment_plan_with_snapshot_hash(
                check_context.intent.as_str(),
                check_context.session_id.as_str(),
                check_context.cursor.as_str(),
                &segment,
                context,
                check_context.pack_snapshot_hash.as_str(),
                check_context.chain_scope.as_slice(),
            ) {
                Ok(plan) => json!({
                    "ok": true,
                    "segment_id": segment.segment_id,
                    "node_count": plan.nodes.len(),
                    "issues": []
                }),
                Err(error) => json!({
                    "ok": false,
                    "segment_id": segment.segment_id,
                    "reason_code": error.get("reason_code").cloned().unwrap_or_else(|| json!("compile_error")),
                    "issues": error.get("issues").cloned().unwrap_or_else(|| Value::Array(vec![])),
                    "error": error
                }),
            };
            let sanitized = sanitize_for_llm_payload(&payload);
            let compacted = compact_json_with_options(
                &sanitized,
                &JsonBudgetOptions {
                    max_depth: 8,
                    max_object_entries: 64,
                    max_array_items: 24,
                    max_string_chars: 2400,
                },
            );
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "plan.check_segment".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "plan.begin" => {
            if finalize_tool != "plan.begin" {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args: BeginToolArgs = serde_json::from_value(tool.arguments.clone())
                .map_err(|error| RunnerError::Llm(format!("invalid plan.begin args: {error}")))?;
            let session_id = coerce_required_scalar_string("session_id", &args.session_id)?;
            let snapshot_hash =
                coerce_required_scalar_string("snapshot_hash", &args.snapshot_hash)?;
            let cursor = coerce_required_scalar_string("cursor", &args.cursor)?;
            Ok(DecodedSegmentedToolCall::Final(PlannerToolOutput::Begin(
                SegmentPlanningSession {
                    session_id,
                    snapshot_hash,
                    cursor,
                    max_rounds: args.limits.max_rounds.max(1),
                    max_segments: args.limits.max_segments.max(1),
                },
            )))
        }
        "plan.ground_intent" => {
            if tool.name != finalize_tool {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args: GroundingToolArgs =
                serde_json::from_value(tool.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid {} args: {error}", tool.name))
                })?;
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::IntentGrounding(parse_grounding_draft(args)?),
            ))
        }
        "plan.propose_todos" => {
            if tool.name != finalize_tool {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args: TodoToolArgs =
                serde_json::from_value(tool.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid {} args: {error}", tool.name))
                })?;
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::TodoDraft(parse_todo_draft(args)?),
            ))
        }
        "plan.propose_segment" | "plan.revise_segment" => {
            if tool.name != finalize_tool {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args: SegmentToolArgs =
                serde_json::from_value(tool.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid {} args: {error}", tool.name))
                })?;
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::SegmentDraft(parse_segment_draft(args)?),
            ))
        }
        other => Err(RunnerError::Llm(format!(
            "unsupported segmented planner tool `{other}`"
        ))),
    }
}

fn phase_from_finalize_tool(finalize_tool: &str) -> Result<PlannerRoundPhase, RunnerError> {
    match finalize_tool {
        "plan.begin" => Ok(PlannerRoundPhase::Begin),
        "plan.ground_intent" => Ok(PlannerRoundPhase::GroundIntent),
        "plan.propose_todos" => Ok(PlannerRoundPhase::ProposeTodos),
        "plan.propose_segment" => Ok(PlannerRoundPhase::ProposeSegment),
        "plan.revise_segment" => Ok(PlannerRoundPhase::ReviseSegment),
        other => Err(RunnerError::Llm(format!(
            "unsupported segmented planner finalize tool `{other}`"
        ))),
    }
}

fn tool_cache_key(tool_name: &str, arguments: &Value) -> Option<String> {
    match tool_name {
        "list_candidates"
        | "get_candidate_detail"
        | "catalog.search"
        | "guide.get"
        | "plan.check_segment" => {
            let normalized = normalize_tool_arguments(tool_name, arguments);
            let hash = stable_hash_hex(&normalized, &StableJsonOptions::default())
                .unwrap_or_else(|_| serde_json::to_string(&normalized).unwrap_or_default());
            Some(format!("{tool_name}:{hash}"))
        }
        _ => None,
    }
}

fn normalize_tool_arguments(tool_name: &str, arguments: &Value) -> Value {
    match tool_name {
        "list_candidates" => json!({}),
        "get_candidate_detail" => {
            let mut refs = arguments
                .get("refs")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            refs.sort();
            refs.dedup();
            json!({ "refs": refs })
        }
        "catalog.search" => arguments.clone(),
        "guide.get" => {
            let schema = arguments
                .get("schema")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_string()))
                .unwrap_or(Value::Null);
            let topic = arguments
                .get("topic")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .unwrap_or(Value::Null);
            json!({
                "schema": schema,
                "topic": topic,
            })
        }
        "plan.check_segment" => json!({
            "segment": arguments.get("segment").cloned().unwrap_or(Value::Null),
        }),
        _ => arguments.clone(),
    }
}

fn guide_get_requires_full_schema(tool_name: &str, arguments: &Value) -> bool {
    if tool_name != "guide.get" {
        return false;
    }
    let full_requested = arguments
        .get("full")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !full_requested {
        return false;
    }
    arguments
        .get("schema")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|schema| !schema.is_empty())
}

fn guide_get_payload_contains_full_schema(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|payload| payload.pointer("/schema/json").cloned())
        .is_some()
}

fn ensure_tool_allowed_for_phase(
    tool_name: &str,
    phase: PlannerRoundPhase,
) -> Result<(), RunnerError> {
    let allowed = match phase {
        PlannerRoundPhase::Begin => matches!(tool_name, "plan.begin"),
        PlannerRoundPhase::GroundIntent => matches!(
            tool_name,
            "list_candidates"
                | "catalog.search"
                | "get_candidate_detail"
                | "guide.get"
                | "plan.ground_intent"
        ),
        PlannerRoundPhase::ProposeTodos => matches!(
            tool_name,
            "list_candidates"
                | "catalog.search"
                | "get_candidate_detail"
                | "guide.get"
                | "plan.propose_todos"
        ),
        PlannerRoundPhase::ProposeSegment => matches!(
            tool_name,
            "list_candidates"
                | "catalog.search"
                | "get_candidate_detail"
                | "guide.get"
                | "plan.check_segment"
                | "plan.propose_segment"
        ),
        PlannerRoundPhase::ReviseSegment => matches!(
            tool_name,
            "list_candidates"
                | "catalog.search"
                | "get_candidate_detail"
                | "guide.get"
                | "plan.check_segment"
                | "plan.revise_segment"
        ),
    };
    if allowed {
        Ok(())
    } else {
        Err(RunnerError::Llm(format!(
            "tool `{tool_name}` is not allowed in planner phase `{}`",
            phase_name(phase)
        )))
    }
}

fn validate_tool_calls_for_phase(
    tool_calls: &[ToolCall],
    phase: PlannerRoundPhase,
) -> Result<(), RunnerError> {
    for call in tool_calls {
        ensure_tool_allowed_for_phase(call.name.as_str(), phase)?;
    }
    let finalize_tool = match phase {
        PlannerRoundPhase::Begin => "plan.begin",
        PlannerRoundPhase::GroundIntent => "plan.ground_intent",
        PlannerRoundPhase::ProposeTodos => "plan.propose_todos",
        PlannerRoundPhase::ProposeSegment => "plan.propose_segment",
        PlannerRoundPhase::ReviseSegment => "plan.revise_segment",
    };
    let finalize_indexes = tool_calls
        .iter()
        .enumerate()
        .filter_map(|(index, call)| (call.name == finalize_tool).then_some(index))
        .collect::<Vec<_>>();

    if phase == PlannerRoundPhase::Begin {
        if tool_calls.len() != 1 || finalize_indexes.len() != 1 {
            return Err(RunnerError::Llm(format!(
                "{} phase requires exactly one tool call: `{finalize_tool}`",
                phase_name(phase)
            )));
        }
        return Ok(());
    }

    if finalize_indexes.len() > 1 {
        return Err(RunnerError::Llm(format!(
            "planner phase `{}` allows at most one finalize tool `{finalize_tool}` per round",
            phase_name(phase)
        )));
    }
    if let Some(index) = finalize_indexes.first() {
        if *index != tool_calls.len().saturating_sub(1) {
            return Err(RunnerError::Llm(format!(
                "finalize tool `{finalize_tool}` must be the last tool call in this round"
            )));
        }
    }
    Ok(())
}

fn phase_name(phase: PlannerRoundPhase) -> &'static str {
    match phase {
        PlannerRoundPhase::Begin => "begin",
        PlannerRoundPhase::GroundIntent => "ground_intent",
        PlannerRoundPhase::ProposeTodos => "propose_todos",
        PlannerRoundPhase::ProposeSegment => "propose_segment",
        PlannerRoundPhase::ReviseSegment => "revise_segment",
    }
}

fn requires_successful_check_before_finalize(
    phase: PlannerRoundPhase,
    segment_check_context: Option<&SegmentCheckContext>,
) -> bool {
    segment_check_context.is_some()
        && matches!(
            phase,
            PlannerRoundPhase::ProposeSegment | PlannerRoundPhase::ReviseSegment
        )
}

fn missing_pre_finalize_check_payload(finalize_tool: &str) -> Value {
    json!({
        "error": {
            "code": "missing_pre_finalize_check_segment",
            "message": format!("call plan.check_segment and wait for ok=true before `{finalize_tool}`"),
            "required_tool": "plan.check_segment",
            "required_ok": true,
            "blocked_finalize": finalize_tool
        }
    })
}

fn plan_check_result_ok(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|value| value.get("ok").and_then(Value::as_bool))
        .unwrap_or(false)
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

fn coerce_required_scalar_string(field: &str, value: &Value) -> Result<String, RunnerError> {
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

fn parse_segment_draft(args: SegmentToolArgs) -> Result<SegmentDraft, RunnerError> {
    match args.status.as_str() {
        "proposed" => {
            let segment_raw = args.segment.ok_or_else(|| {
                RunnerError::Llm("proposed segment draft requires `segment`".to_string())
            })?;
            let segment = decode_plan_sketch_segment_arg(&segment_raw)?;
            let cursor_next = match args.cursor_next {
                Some(cursor_next_raw) => {
                    coerce_required_scalar_string("cursor_next", &cursor_next_raw)?
                }
                None => segment.cursor_out.clone(),
            };
            Ok(SegmentDraft::Proposed {
                summary: args.summary,
                segment,
                cursor_next,
                done: args.done,
                issues: args.issues,
            })
        }
        "unavailable" => {
            let error = args.error.ok_or_else(|| {
                RunnerError::Llm("unavailable segment draft requires `error`".to_string())
            })?;
            let questions =
                extract_missing_input_questions(error.details.as_ref(), args.questions.as_slice());
            Ok(SegmentDraft::Unavailable {
                reason_code: error.reason_code,
                message: error.message,
                done: args.done,
                issues: args.issues,
                questions,
            })
        }
        "invalid" => {
            let error = args.error.ok_or_else(|| {
                RunnerError::Llm("invalid segment draft requires `error`".to_string())
            })?;
            Ok(SegmentDraft::Invalid {
                reason_code: error.reason_code,
                message: error.message,
                done: args.done,
                issues: args.issues,
            })
        }
        other => Err(RunnerError::Llm(format!(
            "invalid segment draft status `{other}`"
        ))),
    }
}

fn parse_todo_draft(args: TodoToolArgs) -> Result<TodoDraft, RunnerError> {
    match args.status.as_str() {
        "proposed" => {
            if args.todos.is_empty() {
                return Err(RunnerError::Llm(
                    "proposed todo draft requires non-empty `todos`".to_string(),
                ));
            }
            Ok(TodoDraft::Proposed {
                summary: args.summary,
                todos: args.todos,
                issues: args.issues,
            })
        }
        "unavailable" => {
            let error = args.error.ok_or_else(|| {
                RunnerError::Llm("unavailable todo draft requires `error`".to_string())
            })?;
            let questions =
                extract_missing_input_questions(error.details.as_ref(), args.questions.as_slice());
            Ok(TodoDraft::Unavailable {
                reason_code: error.reason_code,
                message: error.message,
                issues: args.issues,
                questions,
            })
        }
        "invalid" => {
            let error = args.error.ok_or_else(|| {
                RunnerError::Llm("invalid todo draft requires `error`".to_string())
            })?;
            Ok(TodoDraft::Invalid {
                reason_code: error.reason_code,
                message: error.message,
                issues: args.issues,
            })
        }
        other => Err(RunnerError::Llm(format!(
            "invalid todo draft status `{other}`"
        ))),
    }
}

fn parse_grounding_draft(args: GroundingToolArgs) -> Result<IntentGroundingDraft, RunnerError> {
    match args.status.as_str() {
        "proposed" => {
            let inferred_ready = args
                .ready_for_todos
                .unwrap_or_else(|| args.questions.is_empty() && !args.resolved_inputs.is_empty());
            Ok(IntentGroundingDraft::Proposed {
                summary: args.summary,
                ready_for_todos: inferred_ready,
                resolved_inputs: args.resolved_inputs,
                intent_facts: args.intent_facts,
                confidence: args.confidence,
                issues: args.issues,
                questions: args.questions,
            })
        }
        "unavailable" => {
            let error = args.error.ok_or_else(|| {
                RunnerError::Llm("unavailable grounding draft requires `error`".to_string())
            })?;
            let questions =
                extract_missing_input_questions(error.details.as_ref(), args.questions.as_slice());
            Ok(IntentGroundingDraft::Unavailable {
                reason_code: error.reason_code,
                message: error.message,
                issues: args.issues,
                questions,
            })
        }
        "invalid" => {
            let error = args.error.ok_or_else(|| {
                RunnerError::Llm("invalid grounding draft requires `error`".to_string())
            })?;
            Ok(IntentGroundingDraft::Invalid {
                reason_code: error.reason_code,
                message: error.message,
                issues: args.issues,
            })
        }
        other => Err(RunnerError::Llm(format!(
            "invalid grounding draft status `{other}`"
        ))),
    }
}

fn extract_missing_input_questions(details: Option<&Value>, fallback: &[Value]) -> Vec<Value> {
    if !fallback.is_empty() {
        return fallback.to_vec();
    }
    let Some(raw_questions) = details
        .and_then(|value| value.get("questions"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    raw_questions
        .iter()
        .filter_map(|item| {
            serde_json::from_value::<MissingInputQuestion>(item.clone())
                .ok()
                .and_then(|question| serde_json::to_value(question).ok())
        })
        .collect::<Vec<_>>()
}

fn decode_plan_sketch_segment_arg(raw: &Value) -> Result<PlanSketchSegment, RunnerError> {
    if raw.is_string() {
        return Err(RunnerError::Llm(
            "proposed segment draft `segment` must be a JSON object (stringified JSON is not allowed)"
                .to_string(),
        ));
    }
    if let Some(details) = missing_step_candidate_ref_diagnostics(raw) {
        return Err(RunnerError::Llm(format!(
            "proposed segment draft `segment` is invalid: steps missing required `candidate_ref`: {details}. Every step (query/action/assert/branch) must include candidate_ref; assert/branch also require candidate_ref from discovered candidates."
        )));
    }
    let value = raw.clone();
    serde_json::from_value::<PlanSketchSegment>(value).map_err(|error| {
        RunnerError::Llm(format!(
            "proposed segment draft `segment` must be a valid PlanSketchSegment: {error}"
        ))
    })
}

fn missing_step_candidate_ref_diagnostics(raw: &Value) -> Option<String> {
    let steps = raw.get("steps").and_then(Value::as_array)?;
    let mut missing = Vec::<String>::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step_obj) = step.as_object() else {
            continue;
        };
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
              "properties":{},
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
        "error":{"$ref":"#/definitions/error"}
      },
      "allOf":[
        {
          "if":{"properties":{"status":{"const":"proposed"}},"required":["status"]},
          "then":{"required":["ready_for_todos"]}
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
            "note": "cursor will be normalized to string by runner"
        },
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
                "Use unavailable+missing_required_input with error.details.questions[] when required user inputs are missing."
            ]
        },
        "session_id": request.session.session_id,
        "snapshot_hash": request.session.snapshot_hash,
        "cursor": request.session.cursor,
        "state_summary": request.state_summary,
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
                "When required data is missing, use unavailable + missing_required_input + questions."
            ]
        },
        "session_id": request.session.session_id,
        "snapshot_hash": request.session.snapshot_hash,
        "cursor": request.session.cursor,
        "state_summary": request.state_summary,
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
                "Use status=proposed and include a valid segment object (do not stringify segment JSON).",
                "Keep segment_id/cursor_in/cursor_out and steps as close as possible to your last attempt; only fix missing/wrong fields and types.",
                "If previous_error.last_failed_finalize exists, treat it as baseline draft and patch minimally to satisfy schema/refs/slots.",
                "Fix unknown_input_ref and missing_required_input slot wiring before semantic rewrites.",
                "Never output legacy branch-tree keys (if_true/if_false/then/else/children); branch is encoded by normal flat steps + when/depends_on."
            ]
        },
        "segment_contract": {
            "required_step_fields": ["id", "kind", "candidate_ref", "inputs"],
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
                "gate step must be backed by query facts in the same segment",
                "if token decimals are missing, add decimals query (erc20/decimals or equivalent) before write, or return missing_required_input"
            ]
        },
        "schema_lookup_contract": {
            "rule": "If you are unsure about schema fields or CEL/ValueRef usage, call guide.get before finalizing.",
            "examples": [
                {"schema":"ais-plan-sketch/0.1.0"},
                {"schema":"ais-agent-intent/0.0.1"},
                {"topic":"cel"},
                {"topic":"valueref"}
            ]
        },
        "check_segment_contract": {
            "rule": "Before finalizing proposed/revised segment, you must call plan.check_segment and only finalize when result.ok=true."
        },
        "depends_on_contract": {
            "rule": "depends_on items must reference known step ids in the same segment",
            "examples": ["q_native_balance", "q_token_balance"]
        },
        "failure_contract": {
            "unavailable_or_invalid": {
                "required_fields": ["status", "done", "error.reason_code"],
                "status_enum": ["unavailable", "invalid"]
            },
            "missing_required_input": {
                "when": "status=unavailable and error.reason_code=missing_required_input",
                "required_fields": ["error.details.questions"],
                "question_shape": {
                    "id": "string",
                    "question": "string",
                    "options": [
                        {
                            "label": "string",
                            "description": "string(optional)",
                            "value": "any(optional)"
                        }
                    ]
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
        "state_summary": request.state_summary,
        "previous_error": request.previous_error,
        "last_segment": request.last_segment,
    });
    if let Some(patch) = patch {
        merge_json_patch(&mut payload, patch);
    }
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
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

fn candidate_snapshot(candidate_context: Option<&CandidateContext>) -> Value {
    candidate_context
        .map(|context| {
            let grouped = grouped_candidate_snapshot(context);
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

fn grouped_candidate_snapshot(context: &CandidateContext) -> Value {
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
        let chains = ref_chains
            .get(reference)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();

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
        "protocols": grouped.into_values().collect::<Vec<_>>(),
        "execution_plugins": execution_plugins,
    })
}

fn is_control_semantics_query(query: Option<&str>) -> bool {
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

fn control_semantics_search_hint_payload(
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
mod tests {
    use super::*;
    use ais_llm::{CompleteWithToolsResponse, ScriptedLlmProvider};
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    fn large_catalog_candidate_context(
        action_count: usize,
        query_count: usize,
    ) -> CandidateContext {
        let mut detail_by_ref = BTreeMap::<String, Value>::new();
        let mut actions = Vec::<Value>::with_capacity(action_count);
        let mut queries = Vec::<Value>::with_capacity(query_count);
        let long_desc = "y".repeat(700);

        for index in 0..action_count {
            let reference = format!("demo@0.0.1/action-{index}");
            let action = json!({
                "ref": reference,
                "id": format!("action-{index}"),
                "description": long_desc.as_str(),
                "params": [{"name":"amount","type":"token_amount","required":true}],
                "execution_types": ["evm_call"],
                "execution_chains": ["eip155:*"]
            });
            detail_by_ref.insert(reference.clone(), action.clone());
            actions.push(action);
        }
        for index in 0..query_count {
            let reference = format!("demo@0.0.1/query-{index}");
            let query = json!({
                "ref": reference,
                "id": format!("query-{index}"),
                "description": long_desc.as_str(),
                "params": [{"name":"owner","type":"address","required":true}],
                "returns": [{"name":"balance","type":"uint256"}],
                "execution_types": ["evm_read"],
                "execution_chains": ["eip155:*"]
            });
            detail_by_ref.insert(reference.clone(), query.clone());
            queries.push(query);
        }

        let index_actions = actions
            .iter()
            .map(|action| {
                json!({
                    "kind":"action",
                    "schema_name":"demo@0.0.1",
                    "name": action.get("id").and_then(Value::as_str).unwrap_or_default(),
                    "ref": action.get("ref").and_then(Value::as_str).unwrap_or_default()
                })
            })
            .collect::<Vec<_>>();
        let index_queries = queries
            .iter()
            .map(|query| {
                json!({
                    "kind":"query",
                    "schema_name":"demo@0.0.1",
                    "name": query.get("id").and_then(Value::as_str).unwrap_or_default(),
                    "ref": query.get("ref").and_then(Value::as_str).unwrap_or_default()
                })
            })
            .collect::<Vec<_>>();
        let executable_actions = actions.clone();
        let executable_queries = queries.clone();

        CandidateContext {
            index_candidates: json!({
                "schema":"ais-executable-candidates/0.0.1",
                "level":"name_only",
                "hash":"x",
                "catalog_schema":"ais-catalog/0.0.1",
                "catalog_hash":"y",
                "actions": index_actions,
                "queries": index_queries,
                "execution_plugins":[{"type":"evm_call","chain":"eip155:1"}]
            }),
            detail_by_ref,
            executable_candidates: ais_sdk::ExecutableCandidates {
                schema: "ais-executable-candidates/0.0.1".to_string(),
                created_at: None,
                hash: "x".to_string(),
                catalog_schema: "ais-catalog/0.0.1".to_string(),
                catalog_hash: "y".to_string(),
                pack: None,
                chain_scope: None,
                actions: executable_actions,
                queries: executable_queries,
                execution_plugins: vec![],
            },
            protocols: vec![],
        }
    }

    #[test]
    fn segmented_planner_begin_session_decodes_tool_payload() {
        let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
            assistant_content: Some("begin".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "plan.begin".to_string(),
                arguments: json!({
                    "session_id":"sess-1",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":4,"max_segments":3}
                }),
            }],
        })]);
        let mut planner = LlmSegmentedIntentPlanner::new(provider);
        let session = planner
            .begin_session(SegmentBeginRequest {
                intent: "check and transfer".to_string(),
                pack_snapshot_hash:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
                chain_scope: vec!["eip155:1".to_string()],
            })
            .expect("must decode begin session");
        assert_eq!(session.session_id, "sess-1");
        assert_eq!(session.cursor, "cursor-0");
        assert_eq!(session.max_rounds, 4);
        assert_eq!(session.max_segments, 3);
    }

    #[test]
    fn segmented_planner_begin_session_coerces_numeric_cursor() {
        let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
            assistant_content: Some("begin".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "plan.begin".to_string(),
                arguments: json!({
                    "session_id":"sess-1",
                    "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "cursor":0,
                    "limits":{"max_rounds":4,"max_segments":3}
                }),
            }],
        })]);
        let mut planner = LlmSegmentedIntentPlanner::new(provider);
        let session = planner
            .begin_session(SegmentBeginRequest {
                intent: "check and transfer".to_string(),
                pack_snapshot_hash:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
                chain_scope: vec!["eip155:1".to_string()],
            })
            .expect("must decode begin session");
        assert_eq!(session.cursor, "0");
    }

    #[test]
    fn segmented_planner_propose_segment_roundtrip() {
        let provider = ScriptedLlmProvider::from_responses(vec![
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("list".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-1".to_string(),
                    name: "list_candidates".to_string(),
                    arguments: json!({}),
                }],
            }),
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("propose".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-2".to_string(),
                    name: "plan.propose_segment".to_string(),
                    arguments: json!({
                        "status":"proposed",
                        "done":false,
                        "cursor_next":"cursor-1",
                        "segment":{
                            "segment_id":"seg-1",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "steps":[{
                                "id":"q1",
                                "kind":"query",
                                "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                                "inputs":{"addr":"0x1111111111111111111111111111111111111111"}
                            }]
                        }
                    }),
                }],
            }),
        ]);
        let mut planner = LlmSegmentedIntentPlanner::new(provider)
            .with_candidate_context(Some(CandidateContext::default()));
        let draft = planner
            .propose_segment(SegmentPlanningRequest {
                intent: "read balance".to_string(),
                session: SegmentPlanningSession {
                    session_id: "sess-1".to_string(),
                    snapshot_hash:
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    cursor: "cursor-0".to_string(),
                    max_rounds: 4,
                    max_segments: 3,
                },
                state_summary: None,
                previous_error: None,
                last_segment: None,
            })
            .expect("must decode proposed segment");
        match draft {
            SegmentDraft::Proposed {
                segment,
                cursor_next,
                done,
                ..
            } => {
                assert_eq!(segment.segment_id, "seg-1");
                assert_eq!(cursor_next, "cursor-1");
                assert!(!done);
            }
            _ => panic!("expected proposed draft"),
        }
    }

    #[test]
    fn segmented_planner_propose_segment_rejects_stringified_segment_json() {
        let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
            assistant_content: Some("propose".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-2".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "cursor_next":1,
                    "segment": serde_json::to_string(&json!({
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":false,
                        "steps":[{
                            "id":"q1",
                            "kind":"query",
                            "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                            "inputs":{"addr":"0x1111111111111111111111111111111111111111"}
                        }]
                    }))
                    .expect("segment json string")
                }),
            }],
        })]);
        let mut planner = LlmSegmentedIntentPlanner::new(provider)
            .with_candidate_context(Some(CandidateContext::default()));
        let error = planner
            .propose_segment(SegmentPlanningRequest {
                intent: "read balance".to_string(),
                session: SegmentPlanningSession {
                    session_id: "sess-1".to_string(),
                    snapshot_hash:
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    cursor: "cursor-0".to_string(),
                    max_rounds: 4,
                    max_segments: 3,
                },
                state_summary: None,
                previous_error: None,
                last_segment: None,
            })
            .expect_err("stringified segment must be rejected");
        assert!(error
            .to_string()
            .contains("must be a JSON object (stringified JSON is not allowed)"));
    }

    #[test]
    fn segmented_planner_propose_segment_uses_cursor_out_when_cursor_next_missing() {
        let provider = ScriptedLlmProvider::from_responses(vec![Ok(CompleteWithToolsResponse {
            assistant_content: Some("propose".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-2".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "done":false,
                    "segment":{
                        "segment_id":"seg-1",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-2",
                        "done":false,
                        "steps":[{
                            "id":"q1",
                            "kind":"query",
                            "candidate_ref":"evm-native-utils@0.0.1/native-balance",
                            "inputs":{"addr":"0x1111111111111111111111111111111111111111"}
                        }]
                    }
                }),
            }],
        })]);
        let mut planner = LlmSegmentedIntentPlanner::new(provider)
            .with_candidate_context(Some(CandidateContext::default()));
        let draft = planner
            .propose_segment(SegmentPlanningRequest {
                intent: "read balance".to_string(),
                session: SegmentPlanningSession {
                    session_id: "sess-1".to_string(),
                    snapshot_hash:
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .to_string(),
                    cursor: "cursor-0".to_string(),
                    max_rounds: 4,
                    max_segments: 3,
                },
                state_summary: None,
                previous_error: None,
                last_segment: None,
            })
            .expect("must decode proposed segment");
        match draft {
            SegmentDraft::Proposed {
                cursor_next, done, ..
            } => {
                assert_eq!(cursor_next, "cursor-2");
                assert!(!done);
            }
            _ => panic!("expected proposed draft"),
        }
    }

    #[test]
    fn segmented_planner_blocks_finalize_until_check_segment_ok() {
        let provider = ScriptedLlmProvider::from_responses(vec![
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("begin".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-begin".to_string(),
                    name: "plan.begin".to_string(),
                    arguments: json!({
                        "session_id":"sess-1",
                        "snapshot_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "cursor":"cursor-0",
                        "limits":{"max_rounds":4,"max_segments":3}
                    }),
                }],
            }),
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("finalize without check".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-finalize-before-check".to_string(),
                    name: "plan.propose_segment".to_string(),
                    arguments: json!({
                        "status":"proposed",
                        "done":false,
                        "cursor_next":"cursor-1",
                        "segment":{
                            "segment_id":"seg-1",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "steps":[]
                        }
                    }),
                }],
            }),
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("check".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-check".to_string(),
                    name: "plan.check_segment".to_string(),
                    arguments: json!({
                        "segment":{
                            "segment_id":"seg-1",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "steps":[]
                        }
                    }),
                }],
            }),
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("finalize after check".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-finalize-after-check".to_string(),
                    name: "plan.propose_segment".to_string(),
                    arguments: json!({
                        "status":"proposed",
                        "done":false,
                        "cursor_next":"cursor-1",
                        "segment":{
                            "segment_id":"seg-1",
                            "cursor_in":"cursor-0",
                            "cursor_out":"cursor-1",
                            "done":false,
                            "steps":[]
                        }
                    }),
                }],
            }),
        ]);
        let mut planner = LlmSegmentedIntentPlanner::new(provider)
            .with_candidate_context(Some(CandidateContext::default()));
        let session = planner
            .begin_session(SegmentBeginRequest {
                intent: "read balance".to_string(),
                pack_snapshot_hash:
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                catalog_hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
                chain_scope: vec!["eip155:1".to_string()],
            })
            .expect("begin session");
        let draft = planner
            .propose_segment(SegmentPlanningRequest {
                intent: "read balance".to_string(),
                session,
                state_summary: None,
                previous_error: None,
                last_segment: None,
            })
            .expect("must finalize after check_segment ok");
        match draft {
            SegmentDraft::Proposed {
                segment,
                cursor_next,
                done,
                ..
            } => {
                assert_eq!(segment.segment_id, "seg-1");
                assert_eq!(cursor_next, "cursor-1");
                assert!(!done);
            }
            _ => panic!("expected proposed draft"),
        }
    }

    #[test]
    fn decode_segmented_tool_call_large_catalog_stays_compact_and_budgeted() {
        let context = large_catalog_candidate_context(260, 260);
        let list_call = ToolCall {
            id: "tool-list".to_string(),
            name: "list_candidates".to_string(),
            arguments: json!({}),
        };
        let list_result = decode_segmented_tool_call(
            &list_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
        )
        .expect("list call");
        let list_content = match list_result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("list must return tool message"),
        };
        let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
        assert_eq!(
            list_json
                .pointer("/protocols/0/actions/24")
                .and_then(Value::as_str)
                .map(|value| value.starts_with("[TRUNCATED_ARRAY_ITEMS:")),
            Some(true)
        );
        assert_eq!(
            list_json
                .pointer("/protocols/0/queries/24")
                .and_then(Value::as_str)
                .map(|value| value.starts_with("[TRUNCATED_ARRAY_ITEMS:")),
            Some(true)
        );
        assert_eq!(
            list_json
                .pointer("/protocols/0/actions/0/ref")
                .and_then(Value::as_str),
            Some("demo@0.0.1/action-0")
        );
        assert!(
            list_json
                .pointer("/protocols/0/actions/0/description")
                .is_none(),
            "name-only index cards must not include description"
        );
        let raw_list_content =
            serde_json::to_string(&context.index_candidates).expect("raw index candidates");
        assert!(list_content.len() < raw_list_content.len());

        let search_call = ToolCall {
            id: "tool-search".to_string(),
            name: "catalog.search".to_string(),
            arguments: json!({
                "query":"query-1",
                "kind":"query",
                "chain":"eip155:1",
                "limit":5
            }),
        };
        let search_result = decode_segmented_tool_call(
            &search_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
        )
        .expect("search call");
        let search_content = match search_result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("search must return tool message"),
        };
        let search_json: Value =
            serde_json::from_str(search_content.as_str()).expect("valid search json");
        assert_eq!(
            search_json.get("returned_matches").and_then(Value::as_u64),
            Some(5)
        );
        assert_eq!(
            search_json.get("truncated").and_then(Value::as_bool),
            Some(true)
        );

        let detail_refs = (0..48)
            .map(|index| format!("demo@0.0.1/query-{index}"))
            .collect::<Vec<_>>();
        let detail_call = ToolCall {
            id: "tool-detail".to_string(),
            name: "get_candidate_detail".to_string(),
            arguments: json!({ "refs": detail_refs }),
        };
        let detail_result = decode_segmented_tool_call(
            &detail_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
        )
        .expect("detail call");
        let detail_content = match detail_result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("detail must return tool message"),
        };
        let detail_json: Value =
            serde_json::from_str(detail_content.as_str()).expect("valid detail json");
        assert_eq!(
            detail_json.get("requested_refs").and_then(Value::as_u64),
            Some(48)
        );
        assert_eq!(
            detail_json.get("returned_refs").and_then(Value::as_u64),
            Some(super::super::candidates::DEFAULT_MAX_DETAIL_REFS as u64)
        );
        assert_eq!(
            detail_json.get("truncated").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            detail_json.pointer("/details/0/params/0/name"),
            Some(&json!("owner"))
        );
        assert_eq!(
            detail_json.pointer("/details/0/params/0/type"),
            Some(&json!("address"))
        );
        assert_eq!(
            detail_json.pointer("/details/0/params/0/required"),
            Some(&json!(true))
        );
        assert_eq!(
            detail_json.pointer("/details/0/returns/0/name"),
            Some(&json!("balance"))
        );
        assert_eq!(
            detail_json.pointer("/details/0/returns/0/type"),
            Some(&json!("uint256"))
        );
        let raw_detail_content =
            serde_json::to_string(&context.get_details_for_refs(&detail_refs)).expect("raw detail");
        assert!(detail_content.len() < raw_detail_content.len());
    }

    #[test]
    fn catalog_search_control_semantics_query_returns_guide_hint() {
        let context = large_catalog_candidate_context(8, 8);
        let call = ToolCall {
            id: "tool-search-control".to_string(),
            name: "catalog.search".to_string(),
            arguments: json!({
                "query":"assert",
                "limit":12
            }),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
        )
        .expect("search call");
        let content = match result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("search must return tool message"),
        };
        let value: Value = serde_json::from_str(content.as_str()).expect("valid search json");
        assert_eq!(value.pointer("/query"), Some(&json!("assert")));
        assert_eq!(value.pointer("/returned_matches"), Some(&json!(0)));
        assert_eq!(
            value.pointer("/hint/reason_code"),
            Some(&json!("control_semantics_not_catalog_candidate"))
        );
        assert_eq!(value.pointer("/hint/next_tool"), Some(&json!("guide.get")));
        assert_eq!(
            value.pointer("/hint/guide_requests/0/schema"),
            Some(&json!("ais-plan-sketch/0.1.0"))
        );
        assert_eq!(
            value.pointer("/hint/guide_requests/1/topic"),
            Some(&json!("cel"))
        );
    }

    #[test]
    fn list_candidates_cards_include_minimum_ref_metadata() {
        let context = large_catalog_candidate_context(2, 2);
        let list_call = ToolCall {
            id: "tool-list-meta".to_string(),
            name: "list_candidates".to_string(),
            arguments: json!({}),
        };
        let list_result = decode_segmented_tool_call(
            &list_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
        )
        .expect("list call");
        let list_content = match list_result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("list must return tool message"),
        };
        let list_json: Value = serde_json::from_str(list_content.as_str()).expect("valid json");
        assert_eq!(
            list_json.pointer("/protocols/0/actions/0/ref"),
            Some(&json!("demo@0.0.1/action-0"))
        );
        assert_eq!(
            list_json.pointer("/protocols/0/actions/0/chains/0"),
            Some(&json!("eip155:*"))
        );
        assert_eq!(
            list_json.pointer("/protocols/0/actions/0/required_inputs/0"),
            Some(&json!("amount"))
        );
        assert!(
            list_json.pointer("/protocols/0/actions/0/name").is_none(),
            "compact list cards should not duplicate action name when ref already encodes it"
        );
    }

    #[test]
    fn planning_memory_caches_list_candidates_per_snapshot_scope() {
        let context = large_catalog_candidate_context(8, 8);
        let call = ToolCall {
            id: "tool-list".to_string(),
            name: "list_candidates".to_string(),
            arguments: json!({}),
        };
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("session-1", "snapshot-1");

        let first = decode_segmented_tool_call_with_memory(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
            None,
            Some(&mut memory),
        )
        .expect("first list");
        let second = decode_segmented_tool_call_with_memory(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
            None,
            Some(&mut memory),
        )
        .expect("second list");

        let (first_content, first_cached) = match first {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };
        let (second_content, second_cached) = match second {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };
        assert!(!first_cached);
        assert!(second_cached);
        assert_eq!(first_content, second_content);

        memory.ensure_scope("session-2", "snapshot-1");
        let third_same_snapshot = decode_segmented_tool_call_with_memory(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
            None,
            Some(&mut memory),
        )
        .expect("third list in same snapshot");
        match third_same_snapshot {
            DecodedSegmentedToolCall::ToolMessage { cached, .. } => assert!(cached),
            _ => panic!("must return tool message"),
        }

        memory.ensure_scope("session-3", "snapshot-2");
        let fourth_new_snapshot = decode_segmented_tool_call_with_memory(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
            None,
            Some(&mut memory),
        )
        .expect("fourth list after snapshot reset");
        match fourth_new_snapshot {
            DecodedSegmentedToolCall::ToolMessage { cached, .. } => assert!(!cached),
            _ => panic!("must return tool message"),
        }
    }

    #[test]
    fn planning_memory_normalizes_detail_ref_order_for_cache_key() {
        let context = large_catalog_candidate_context(2, 6);
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("session-1", "snapshot-1");
        let call_first = ToolCall {
            id: "tool-detail-1".to_string(),
            name: "get_candidate_detail".to_string(),
            arguments: json!({
                "refs": ["demo@0.0.1/query-3", "demo@0.0.1/query-1"]
            }),
        };
        let call_second = ToolCall {
            id: "tool-detail-2".to_string(),
            name: "get_candidate_detail".to_string(),
            arguments: json!({
                "refs": ["demo@0.0.1/query-1", "demo@0.0.1/query-3"]
            }),
        };

        let first = decode_segmented_tool_call_with_memory(
            &call_first,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
            None,
            Some(&mut memory),
        )
        .expect("first detail");
        let second = decode_segmented_tool_call_with_memory(
            &call_second,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&context),
            None,
            Some(&mut memory),
        )
        .expect("second detail");

        let (first_content, first_cached) = match first {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };
        let (second_content, second_cached) = match second {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };
        assert!(!first_cached);
        assert!(second_cached);
        assert_eq!(first_content, second_content);
    }

    #[test]
    fn planning_memory_guide_get_full_request_refreshes_digest_cache_entry() {
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("session-1", "snapshot-1");
        let digest_call = ToolCall {
            id: "tool-guide-digest".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({
                "schema": "ais-plan-sketch/0.1.0"
            }),
        };
        let full_call = ToolCall {
            id: "tool-guide-full".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({
                "schema": "ais-plan-sketch/0.1.0",
                "full": true
            }),
        };

        let first = decode_segmented_tool_call_with_memory(
            &digest_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
            None,
            Some(&mut memory),
        )
        .expect("digest schema lookup");
        let second = decode_segmented_tool_call_with_memory(
            &full_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
            None,
            Some(&mut memory),
        )
        .expect("full schema lookup");
        let third = decode_segmented_tool_call_with_memory(
            &full_call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
            None,
            Some(&mut memory),
        )
        .expect("cached full schema lookup");

        let (first_content, first_cached) = match first {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };
        let (second_content, second_cached) = match second {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };
        let (third_content, third_cached) = match third {
            DecodedSegmentedToolCall::ToolMessage {
                content, cached, ..
            } => (content, cached),
            _ => panic!("must return tool message"),
        };

        let first_json = serde_json::from_str::<Value>(first_content.as_str()).expect("json");
        let second_json = serde_json::from_str::<Value>(second_content.as_str()).expect("json");
        let third_json = serde_json::from_str::<Value>(third_content.as_str()).expect("json");

        assert!(!first_cached);
        assert_eq!(first_json.pointer("/schema/mode"), Some(&json!("digest")));
        assert!(first_json.pointer("/schema/json").is_none());

        assert!(!second_cached);
        assert_eq!(second_json.pointer("/schema/mode"), Some(&json!("full")));
        assert!(second_json.pointer("/schema/json").is_some());

        assert!(third_cached);
        assert_eq!(third_json.pointer("/schema/mode"), Some(&json!("full")));
        assert!(third_json.pointer("/schema/json").is_some());
    }

    #[test]
    fn phase_tools_are_scoped_by_round() {
        let begin_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::Begin)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(begin_tools, BTreeSet::from_iter(["plan.begin".to_string()]));

        let grounding_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::GroundIntent)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert!(grounding_tools.contains("list_candidates"));
        assert!(grounding_tools.contains("catalog.search"));
        assert!(grounding_tools.contains("get_candidate_detail"));
        assert!(grounding_tools.contains("guide.get"));
        assert!(grounding_tools.contains("plan.ground_intent"));
        assert!(!grounding_tools.contains("plan.begin"));
        assert!(!grounding_tools.contains("plan.propose_todos"));
        assert!(!grounding_tools.contains("plan.propose_segment"));
        assert!(!grounding_tools.contains("plan.revise_segment"));

        let todos_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeTodos)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert!(todos_tools.contains("list_candidates"));
        assert!(todos_tools.contains("catalog.search"));
        assert!(todos_tools.contains("get_candidate_detail"));
        assert!(todos_tools.contains("guide.get"));
        assert!(todos_tools.contains("plan.propose_todos"));
        assert!(!todos_tools.contains("plan.begin"));
        assert!(!todos_tools.contains("plan.propose_segment"));
        assert!(!todos_tools.contains("plan.revise_segment"));

        let propose_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeSegment)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert!(propose_tools.contains("list_candidates"));
        assert!(propose_tools.contains("catalog.search"));
        assert!(propose_tools.contains("get_candidate_detail"));
        assert!(propose_tools.contains("guide.get"));
        assert!(propose_tools.contains("plan.check_segment"));
        assert!(propose_tools.contains("plan.propose_segment"));
        assert!(!propose_tools.contains("plan.propose_todos"));
        assert!(!propose_tools.contains("plan.begin"));
        assert!(!propose_tools.contains("plan.revise_segment"));

        let revise_tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ReviseSegment)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert!(revise_tools.contains("list_candidates"));
        assert!(revise_tools.contains("catalog.search"));
        assert!(revise_tools.contains("get_candidate_detail"));
        assert!(revise_tools.contains("guide.get"));
        assert!(revise_tools.contains("plan.check_segment"));
        assert!(revise_tools.contains("plan.revise_segment"));
        assert!(!revise_tools.contains("plan.propose_todos"));
        assert!(!revise_tools.contains("plan.begin"));
        assert!(!revise_tools.contains("plan.propose_segment"));
    }

    #[test]
    fn plan_propose_todos_tool_schema_requires_todo_title() {
        let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeTodos);
        let schema = tools
            .into_iter()
            .find(|tool| tool.name == "plan.propose_todos")
            .map(|tool| tool.input_schema)
            .expect("plan.propose_todos schema");
        assert_eq!(
            schema.pointer("/properties/status/enum/0"),
            Some(&json!("proposed"))
        );
        assert_eq!(
            schema.pointer("/properties/todos/items/$ref"),
            Some(&json!("#/$defs/todo_item"))
        );
        assert_eq!(
            schema.pointer("/$defs/todo_item/required/0"),
            Some(&json!("title"))
        );
    }

    #[test]
    fn propose_todo_draft_roundtrip_decodes_todos() {
        let call = ToolCall {
            id: "tool-todos-final".to_string(),
            name: "plan.propose_todos".to_string(),
            arguments: json!({
                "status": "proposed",
                "summary": "split into 2 todos",
                "todos": [
                    {
                        "title": "Query token decimals",
                        "required_facts": ["token.address"],
                        "produced_facts": ["token.decimals"]
                    },
                    {
                        "title": "Execute transfer",
                        "required_facts": ["token.decimals", "amount.human"],
                        "produced_facts": ["tx_hash"]
                    }
                ]
            }),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.propose_todos",
            PlannerRoundPhase::ProposeTodos,
            None,
        )
        .expect("todo finalize call");
        match result {
            DecodedSegmentedToolCall::Final(PlannerToolOutput::TodoDraft(
                TodoDraft::Proposed { summary, todos, .. },
            )) => {
                assert_eq!(summary.as_deref(), Some("split into 2 todos"));
                assert_eq!(todos.len(), 2);
                assert_eq!(todos[0].title, "Query token decimals");
                assert_eq!(todos[1].produced_facts, vec!["tx_hash".to_string()]);
            }
            _ => panic!("expected proposed todo draft"),
        }
    }

    #[test]
    fn ground_intent_tool_schema_requires_ready_for_todos() {
        let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::GroundIntent);
        let schema = tools
            .into_iter()
            .find(|tool| tool.name == "plan.ground_intent")
            .map(|tool| tool.input_schema)
            .expect("plan.ground_intent schema");
        assert_eq!(
            schema.pointer("/properties/status/enum/0"),
            Some(&json!("proposed"))
        );
        assert_eq!(
            schema.pointer("/allOf/0/then/required/0"),
            Some(&json!("ready_for_todos"))
        );
    }

    #[test]
    fn grounding_draft_roundtrip_decodes_proposed_payload() {
        let call = ToolCall {
            id: "tool-grounding-final".to_string(),
            name: "plan.ground_intent".to_string(),
            arguments: json!({
                "status": "proposed",
                "summary": "extracted transfer fields",
                "ready_for_todos": true,
                "resolved_inputs": {
                    "owner": "0x1111",
                    "recipient": "0x2222",
                    "amount": "1.25"
                },
                "intent_facts": {
                    "intent.action": "transfer"
                },
                "confidence": {
                    "owner": 95,
                    "recipient": 93
                }
            }),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.ground_intent",
            PlannerRoundPhase::GroundIntent,
            None,
        )
        .expect("grounding finalize call");
        match result {
            DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
                IntentGroundingDraft::Proposed {
                    ready_for_todos,
                    resolved_inputs,
                    intent_facts,
                    ..
                },
            )) => {
                assert!(ready_for_todos);
                assert_eq!(
                    resolved_inputs.get("recipient").and_then(Value::as_str),
                    Some("0x2222")
                );
                assert_eq!(
                    intent_facts.get("intent.action").and_then(Value::as_str),
                    Some("transfer")
                );
            }
            _ => panic!("expected proposed grounding draft"),
        }
    }

    #[test]
    fn grounding_draft_infers_ready_when_flag_missing_and_no_questions() {
        let call = ToolCall {
            id: "tool-grounding-final".to_string(),
            name: "plan.ground_intent".to_string(),
            arguments: json!({
                "status": "proposed",
                "summary": "extracted transfer fields",
                "resolved_inputs": {
                    "owner": "0x1111",
                    "recipient": "0x2222"
                },
                "intent_facts": {},
                "confidence": {
                    "owner": 95,
                    "recipient": 93
                }
            }),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.ground_intent",
            PlannerRoundPhase::GroundIntent,
            None,
        )
        .expect("grounding finalize call");
        match result {
            DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
                IntentGroundingDraft::Proposed {
                    ready_for_todos, ..
                },
            )) => {
                assert!(ready_for_todos);
            }
            _ => panic!("expected proposed grounding draft"),
        }
    }

    #[test]
    fn grounding_draft_keeps_not_ready_when_flag_missing_and_questions_exist() {
        let call = ToolCall {
            id: "tool-grounding-final".to_string(),
            name: "plan.ground_intent".to_string(),
            arguments: json!({
                "status": "proposed",
                "summary": "need more inputs",
                "resolved_inputs": {
                    "owner": "0x1111"
                },
                "questions": [
                    {"id":"recipient","question":"recipient?"}
                ],
                "confidence": {
                    "owner": 95
                }
            }),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.ground_intent",
            PlannerRoundPhase::GroundIntent,
            None,
        )
        .expect("grounding finalize call");
        match result {
            DecodedSegmentedToolCall::Final(PlannerToolOutput::IntentGrounding(
                IntentGroundingDraft::Proposed {
                    ready_for_todos, ..
                },
            )) => {
                assert!(!ready_for_todos);
            }
            _ => panic!("expected proposed grounding draft"),
        }
    }

    #[test]
    fn segment_draft_tool_schema_requires_step_id() {
        let step_schema = propose_segment_step_schema();
        let required = step_schema
            .get("required")
            .and_then(Value::as_array)
            .expect("required fields");
        let required_set = required
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        assert!(required_set.contains("id"));
        assert!(required_set.contains("kind"));
        assert!(required_set.contains("candidate_ref"));
        assert!(required_set.contains("inputs"));
    }

    #[test]
    fn segment_draft_tool_schema_includes_runtime_controls() {
        let step_schema = propose_segment_step_schema();
        let step_props = step_schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("step properties");
        assert!(step_props.contains_key("until"));
        assert!(step_props.contains_key("retry"));
        assert!(step_props.contains_key("timeout_ms"));
    }

    #[test]
    fn segment_draft_tool_schema_accepts_control_step_kinds() {
        let step_schema = propose_segment_step_schema();
        let kind_enum = step_schema
            .pointer("/properties/kind/enum")
            .and_then(Value::as_array)
            .expect("kind enum");
        let kind_set = kind_enum
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        assert!(kind_set.contains("action"));
        assert!(kind_set.contains("query"));
        assert!(kind_set.contains("assert"));
        assert!(kind_set.contains("branch"));
    }

    #[test]
    fn plan_check_segment_tool_returns_compile_issues_without_candidate_match() {
        let call = ToolCall {
            id: "tool-check-segment".to_string(),
            name: "plan.check_segment".to_string(),
            arguments: json!({
                "segment": {
                    "segment_id": "seg-check",
                    "cursor_in": "c0",
                    "cursor_out": "c1",
                    "done": false,
                    "steps": [{
                        "id": "q1",
                        "kind": "query",
                        "candidate_ref": "missing@0.0.1/query",
                        "inputs": {}
                    }]
                }
            }),
        };
        let check_context = SegmentCheckContext {
            intent: "check segment".to_string(),
            session_id: "s-1".to_string(),
            cursor: "0".to_string(),
            pack_snapshot_hash: "a".repeat(64),
            chain_scope: vec!["eip155:1".to_string()],
        };
        let result = decode_segmented_tool_call_with_memory(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            Some(&CandidateContext::default()),
            Some(&check_context),
            None,
        )
        .expect("check call");
        let content = match result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("check must return tool message"),
        };
        let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
        assert_eq!(value.pointer("/ok"), Some(&json!(false)));
        assert_eq!(value.pointer("/reason_code"), Some(&json!("compile_error")));
        let issues = value
            .pointer("/issues")
            .and_then(Value::as_array)
            .expect("issues array");
        assert!(!issues.is_empty());
    }

    fn propose_segment_step_schema() -> Value {
        let tools = segmented_planner_tools_for_phase(PlannerRoundPhase::ProposeSegment);
        let schema = tools
            .into_iter()
            .find(|tool| tool.name == "plan.propose_segment")
            .map(|tool| tool.input_schema)
            .expect("plan.propose_segment schema");
        let segment_ref = schema
            .pointer("/properties/segment/$ref")
            .and_then(Value::as_str)
            .expect("segment ref");
        let segment_schema = schema
            .pointer(segment_ref.trim_start_matches('#'))
            .cloned()
            .expect("segment schema");
        let step_ref = segment_schema
            .pointer("/properties/steps/items/$ref")
            .and_then(Value::as_str)
            .expect("steps item ref");
        schema
            .pointer(step_ref.trim_start_matches('#'))
            .cloned()
            .expect("segment step schema")
    }

    #[test]
    fn guide_get_tool_returns_topic_guide() {
        let call = ToolCall {
            id: "tool-schema-topic".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({"topic":"cel"}),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
        )
        .expect("guide.get topic call");
        let content = match result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("guide.get must return tool message"),
        };
        let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
        assert_eq!(value.get("kind"), Some(&json!("topic")));
        assert_eq!(value.pointer("/topic/topic"), Some(&json!("cel")));
        assert_eq!(
            value.pointer("/topic/allowed_namespaces/0"),
            Some(&json!("inputs"))
        );
    }

    #[test]
    fn guide_get_tool_returns_embedded_schema() {
        let call = ToolCall {
            id: "tool-schema-id".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({"schema":"ais-plan-sketch/0.1.0"}),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
        )
        .expect("guide.get schema_id call");
        let content = match result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("guide.get must return tool message"),
        };
        let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
        assert_eq!(value.get("kind"), Some(&json!("schema")));
        assert_eq!(
            value.pointer("/schema/id"),
            Some(&json!("ais-plan-sketch/0.1.0"))
        );
        assert_eq!(value.pointer("/schema/mode"), Some(&json!("digest")));
        assert!(value.pointer("/schema/digest").is_some());
        assert!(value.pointer("/schema/json").is_none());
    }

    #[test]
    fn guide_get_tool_returns_full_schema_when_requested() {
        let call = ToolCall {
            id: "tool-schema-id-full".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({"schema":"ais-plan-sketch/0.1.0","full":true}),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
        )
        .expect("guide.get full schema call");
        let content = match result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("guide.get must return tool message"),
        };
        let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
        assert_eq!(value.get("kind"), Some(&json!("schema")));
        assert_eq!(value.pointer("/schema/mode"), Some(&json!("full")));
        assert!(value.pointer("/schema/digest").is_some());
        assert!(value.pointer("/schema/json").is_some());
    }

    #[test]
    fn guide_get_tool_rejects_object_schema_arg() {
        let call = ToolCall {
            id: "tool-guide-schema-object".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({
                "schema": {"id":"ais-plan-sketch/0.1.0"}
            }),
        };
        let error = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
        )
        .expect_err("object schema arg must be rejected");
        assert!(error.to_string().contains("invalid guide.get args"));
    }

    #[test]
    fn guide_get_tool_rejects_object_topic_arg() {
        let call = ToolCall {
            id: "tool-guide-topic-object".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({
                "topic": {"name":"cel"}
            }),
        };
        let error = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
        )
        .expect_err("object topic arg must be rejected");
        assert!(error.to_string().contains("invalid guide.get args"));
    }

    #[test]
    fn guide_get_tool_rejects_stringified_schema_object_arg() {
        let call = ToolCall {
            id: "tool-guide-schema-stringified-object".to_string(),
            name: "guide.get".to_string(),
            arguments: json!({
                "schema": "{\"id\":\"ais-plan-sketch/0.1.0\"}"
            }),
        };
        let result = decode_segmented_tool_call(
            &call,
            "plan.propose_segment",
            PlannerRoundPhase::ProposeSegment,
            None,
        )
        .expect("guide.get stringified object schema call");
        let content = match result {
            DecodedSegmentedToolCall::ToolMessage { content, .. } => content,
            _ => panic!("guide.get must return tool message"),
        };
        let value: Value = serde_json::from_str(content.as_str()).expect("json payload");
        assert_eq!(value.pointer("/kind"), Some(&json!("schema")));
        assert_eq!(
            value.pointer("/error/code"),
            Some(&json!("schema_not_found"))
        );
    }

    #[test]
    fn guide_get_tool_schema_prefers_canonical_string_request_shape() {
        let schema = segmented_planner_tools()
            .into_iter()
            .find(|tool| tool.name == "guide.get")
            .map(|tool| tool.input_schema)
            .expect("guide.get schema");
        assert_eq!(
            schema.pointer("/oneOf/0/properties/schema/type"),
            Some(&json!("string"))
        );
        assert_eq!(
            schema.pointer("/oneOf/0/properties/full/type"),
            Some(&json!("boolean"))
        );
        assert_eq!(
            schema.pointer("/oneOf/1/properties/topic/enum/0"),
            Some(&json!("cel"))
        );
        assert_eq!(
            schema.pointer("/oneOf/1/properties/topic/enum/1"),
            Some(&json!("valueref"))
        );
        assert!(
            schema.pointer("/oneOf/1/properties/topic/enum/2").is_none(),
            "guide topic enum should not expose constraint_templates"
        );
    }

    #[test]
    fn guide_get_cache_key_uses_canonical_string_shapes_only() {
        let schema_from_canonical =
            tool_cache_key("guide.get", &json!({"schema":"ais-plan-sketch/0.1.0"}))
                .expect("canonical schema cache key");
        let schema_from_object = tool_cache_key(
            "guide.get",
            &json!({"schema":{"id":"ais-plan-sketch/0.1.0"}}),
        )
        .expect("object schema cache key");
        assert_ne!(schema_from_canonical, schema_from_object);

        let topic_from_canonical = tool_cache_key("guide.get", &json!({"topic":"cel"}))
            .expect("canonical topic cache key");
        let topic_from_object = tool_cache_key("guide.get", &json!({"topic":{"name":"cel"}}))
            .expect("object topic cache key");
        assert_ne!(topic_from_canonical, topic_from_object);
    }

    #[test]
    fn requires_successful_check_only_for_segment_finalize_with_context() {
        assert!(!requires_successful_check_before_finalize(
            PlannerRoundPhase::Begin,
            None
        ));
        assert!(!requires_successful_check_before_finalize(
            PlannerRoundPhase::ProposeSegment,
            None
        ));
        let context = SegmentCheckContext {
            intent: "i".to_string(),
            session_id: "s".to_string(),
            cursor: "0".to_string(),
            pack_snapshot_hash: "a".repeat(64),
            chain_scope: vec!["eip155:1".to_string()],
        };
        assert!(requires_successful_check_before_finalize(
            PlannerRoundPhase::ProposeSegment,
            Some(&context)
        ));
        assert!(requires_successful_check_before_finalize(
            PlannerRoundPhase::ReviseSegment,
            Some(&context)
        ));
        assert!(!requires_successful_check_before_finalize(
            PlannerRoundPhase::ProposeTodos,
            Some(&context)
        ));
    }

    #[test]
    fn unavailable_draft_extracts_missing_input_questions_from_error_details() {
        let draft = parse_segment_draft(SegmentToolArgs {
            status: "unavailable".to_string(),
            done: false,
            segment: None,
            summary: None,
            cursor_next: None,
            issues: Vec::new(),
            error: Some(SegmentError {
                reason_code: "missing_required_input".to_string(),
                message: Some("missing owner".to_string()),
                details: Some(json!({
                    "questions": [
                        {
                            "id": "owner",
                            "question": "who is the owner",
                            "options": [
                                {"label": "wallet-1", "value": "0xabc"}
                            ]
                        }
                    ]
                })),
            }),
            questions: Vec::new(),
        })
        .expect("parse unavailable draft");
        match draft {
            SegmentDraft::Unavailable {
                reason_code,
                questions,
                ..
            } => {
                assert_eq!(reason_code, "missing_required_input");
                assert_eq!(questions.len(), 1);
                assert_eq!(questions[0].pointer("/id"), Some(&json!("owner")));
                assert_eq!(
                    questions[0].pointer("/options/0/label"),
                    Some(&json!("wallet-1"))
                );
            }
            _ => panic!("draft must be unavailable"),
        }
    }

    #[test]
    fn render_segment_prompt_uses_detect_free_valueref_and_contracts() {
        let prompt = render_segment_prompt(
            "plan.propose_segment",
            &SegmentPlanningRequest {
                intent: "transfer token".to_string(),
                session: SegmentPlanningSession {
                    session_id: "s".to_string(),
                    snapshot_hash: "h".to_string(),
                    cursor: "0".to_string(),
                    max_rounds: 6,
                    max_segments: 8,
                },
                state_summary: None,
                previous_error: None,
                last_segment: None,
            },
        );
        let value: Value = serde_json::from_str(prompt.as_str()).expect("prompt json");
        let allowed = value
            .pointer("/value_ref_contract/allowed")
            .and_then(Value::as_array)
            .expect("allowed ValueRef kinds");
        assert!(!allowed.iter().any(|item| item == "detect"));
        assert_eq!(
            value.pointer("/asset_param_contract/rule"),
            Some(&json!(
                "for param type=asset, input must resolve to object with address"
            ))
        );
        assert_eq!(
            value.pointer("/segment_contract/optional_runtime_controls/0"),
            Some(&json!("until"))
        );
        assert_eq!(
            value.pointer("/segment_contract/required_step_fields/2"),
            Some(&json!("candidate_ref"))
        );
        assert_eq!(
            value.pointer("/segment_contract/kind_enum/2"),
            Some(&json!("assert"))
        );
        assert_eq!(
            value.pointer("/segment_contract/kind_enum/3"),
            Some(&json!("branch"))
        );
        assert_eq!(
            value.pointer("/depends_on_contract/rule"),
            Some(&json!(
                "depends_on items must reference known step ids in the same segment"
            ))
        );
        assert_eq!(
            value.pointer("/depends_on_contract/examples/1"),
            Some(&json!("q_token_balance"))
        );
        assert_eq!(
            value.pointer("/schema_lookup_contract/examples/0/schema"),
            Some(&json!("ais-plan-sketch/0.1.0"))
        );
        assert_eq!(
            value.pointer("/schema_lookup_contract/examples/2/topic"),
            Some(&json!("cel"))
        );
        assert_eq!(
            value.pointer("/failure_contract/missing_required_input/required_fields/0"),
            Some(&json!("error.details.questions"))
        );
        assert_eq!(
            value
                .pointer("/failure_contract/missing_required_input/question_shape/options/0/label"),
            Some(&json!("string"))
        );
        assert!(
            value
                .pointer("/schema_lookup_contract/examples/0/schema/id")
                .is_none(),
            "schema lookup examples must use canonical string shape"
        );
        assert!(
            !prompt.contains("seg_1/"),
            "prompt must not encourage cross-segment depends_on references"
        );
    }

    #[test]
    fn decode_plan_sketch_segment_arg_reports_missing_candidate_ref_with_step_context() {
        let error = decode_plan_sketch_segment_arg(&json!({
            "segment_id":"seg_1",
            "cursor_in":"0",
            "cursor_out":"1",
            "done":false,
            "steps":[
                {"id":"q_balance","kind":"query","inputs":{}},
                {"id":"a_guard","kind":"assert","inputs":{}},
                {"id":"a_tx","kind":"action","candidate_ref":"erc20@0.0.2/transfer","inputs":{}}
            ]
        }))
        .expect_err("missing candidate_ref must fail");
        let message = error.to_string();
        assert!(message.contains("missing required `candidate_ref`"));
        assert!(message.contains("q_balance(query)"));
        assert!(message.contains("a_guard(assert)"));
    }

    #[test]
    fn render_segment_prompt_with_patch_overrides_nested_fields() {
        let request = SegmentPlanningRequest {
            intent: "transfer token".to_string(),
            session: SegmentPlanningSession {
                session_id: "s".to_string(),
                snapshot_hash: "h".to_string(),
                cursor: "0".to_string(),
                max_rounds: 6,
                max_segments: 8,
            },
            state_summary: None,
            previous_error: None,
            last_segment: None,
        };
        let patch = json!({
            "segment_contract": {
                "notes": "patched-note"
            },
            "custom_hint": "x"
        });
        let prompt =
            render_segment_prompt_with_patch("plan.propose_segment", &request, Some(&patch));
        let value: Value = serde_json::from_str(prompt.as_str()).expect("prompt json");
        assert_eq!(
            value.pointer("/segment_contract/notes"),
            Some(&json!("patched-note"))
        );
        assert_eq!(value.pointer("/custom_hint"), Some(&json!("x")));
    }

    #[test]
    fn system_prompt_builder_emits_stable_version_and_hash() {
        let builder = SegmentedPromptContextBuilder::default();
        let rendered_a = builder.render(PlannerRoundPhase::ProposeSegment, None);
        let rendered_b = builder.render(PlannerRoundPhase::ProposeSegment, None);
        assert_eq!(rendered_a.version, SEGMENTED_PROMPT_VERSION);
        assert_eq!(rendered_a.hash, rendered_b.hash);
        assert!(rendered_a
            .prompt
            .contains("Prompt-Version: aisrs-segmented-planner-v2"));
        assert!(rendered_a.prompt.contains("Prompt-Hash: "));
    }

    #[test]
    fn usage_tracker_records_estimated_tokens() {
        let request = CompleteWithToolsRequest {
            messages: vec![LlmMessage {
                role: MessageRole::User,
                content: Some("hello".to_string()),
                tool_name: None,
                tool_call_id: None,
                tool_calls: vec![],
            }],
            tools: vec![],
        };
        let response = ais_llm::CompleteWithToolsResponse {
            assistant_content: Some("world".to_string()),
            tool_calls: vec![],
        };
        let mut tracker = PlannerLlmUsageTracker::default().with_context_limit_tokens(Some(1000));
        let usage = tracker.record_estimated(&request, &response);
        assert!(usage.input_tokens > 0);
        assert!(usage.output_tokens > 0);
        assert!(usage.total_tokens >= usage.input_tokens);
        assert!(usage.estimated);
        assert_eq!(usage.context_limit_tokens, Some(1000));
        assert_eq!(usage.context_soft_limit_tokens, Some(900));
        assert_eq!(
            usage.context_remaining_tokens,
            Some(900_u64.saturating_sub(usage.input_tokens))
        );
        let value = tracker.to_value();
        assert_eq!(value.pointer("/calls"), Some(&json!(1)));
        assert_eq!(
            value.pointer("/source"),
            Some(&json!("estimated(chars_div_4)"))
        );
        assert_eq!(value.pointer("/context_limit_tokens"), Some(&json!(1000)));
        assert_eq!(
            value.pointer("/context_soft_limit_tokens"),
            Some(&json!(900))
        );
        assert_eq!(
            value.pointer("/context_window_input_tokens"),
            Some(&json!(usage.input_tokens))
        );
        assert_eq!(
            value.pointer("/context_window_total_tokens"),
            Some(&json!(usage.total_tokens))
        );
        assert_eq!(
            value.pointer("/context_remaining_tokens"),
            Some(&json!(900_u64.saturating_sub(usage.input_tokens)))
        );
    }

    #[test]
    fn begin_phase_rejects_discovery_tools() {
        let calls = vec![ToolCall {
            id: "tool-1".to_string(),
            name: "list_candidates".to_string(),
            arguments: json!({}),
        }];
        let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::Begin)
            .expect_err("begin phase should reject discovery tools");
        assert!(error.to_string().contains("not allowed"));
    }

    #[test]
    fn propose_phase_rejects_revise_tool() {
        let calls = vec![ToolCall {
            id: "tool-1".to_string(),
            name: "plan.revise_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        }];
        let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
            .expect_err("propose phase should reject revise tool");
        assert!(error.to_string().contains("not allowed"));
    }

    #[test]
    fn todo_phase_rejects_segment_finalize_tool() {
        let calls = vec![ToolCall {
            id: "tool-1".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        }];
        let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeTodos)
            .expect_err("todo phase should reject segment finalize tool");
        assert!(error.to_string().contains("not allowed"));
    }

    #[test]
    fn todo_phase_allows_discovery_then_finalize() {
        let calls = vec![
            ToolCall {
                id: "tool-1".to_string(),
                name: "list_candidates".to_string(),
                arguments: json!({}),
            },
            ToolCall {
                id: "tool-2".to_string(),
                name: "plan.propose_todos".to_string(),
                arguments: json!({
                    "status":"proposed",
                    "todos":[{"title":"t1"}]
                }),
            },
        ];
        validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeTodos)
            .expect("todo phase should allow discovery + finalize");
    }

    #[test]
    fn revise_phase_rejects_propose_tool() {
        let calls = vec![ToolCall {
            id: "tool-1".to_string(),
            name: "plan.propose_segment".to_string(),
            arguments: json!({
                "status":"invalid",
                "done":false,
                "error":{"reason_code":"x"}
            }),
        }];
        let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ReviseSegment)
            .expect_err("revise phase should reject propose tool");
        assert!(error.to_string().contains("not allowed"));
    }

    #[test]
    fn finalize_tool_must_be_last_in_round() {
        let calls = vec![
            ToolCall {
                id: "tool-1".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":false,
                    "error":{"reason_code":"x"}
                }),
            },
            ToolCall {
                id: "tool-2".to_string(),
                name: "get_candidate_detail".to_string(),
                arguments: json!({
                    "refs":["demo@0.0.1/action-1"]
                }),
            },
        ];
        let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
            .expect_err("finalize tool should be last");
        assert!(error.to_string().contains("must be the last tool call"));
    }

    #[test]
    fn finalize_tool_at_most_once_per_round() {
        let calls = vec![
            ToolCall {
                id: "tool-1".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":false,
                    "error":{"reason_code":"x"}
                }),
            },
            ToolCall {
                id: "tool-2".to_string(),
                name: "plan.propose_segment".to_string(),
                arguments: json!({
                    "status":"invalid",
                    "done":false,
                    "error":{"reason_code":"x"}
                }),
            },
        ];
        let error = validate_tool_calls_for_phase(&calls, PlannerRoundPhase::ProposeSegment)
            .expect_err("finalize tool should appear at most once");
        assert!(error.to_string().contains("at most one finalize tool"));
    }
}
