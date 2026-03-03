use super::super::budget::compact_json_with_options;
use super::budget_policy::{ContextCompactionPolicy, ContextPressureMode, ToolMemoryBudgetPolicy};
use super::packing::{
    ContextBlockPriority, ContextCompressLevel, ContextPackBlockId, PackAction, PackDecision,
    PackDiagnostics, PackTrace,
};
use ais_engine::EngineRunnerState;
use serde_json::{json, Value};

pub(in super::super) const DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET: usize =
    ToolMemoryBudgetPolicy::PLANNER_CONTEXT_DEFAULT_TOKEN_BUDGET;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

#[derive(Debug, Clone, Copy)]
struct AdaptivePlannerBudget {
    effective_token_limit: usize,
    remaining_tokens: Option<u64>,
    usage_ratio_bps: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct ContextCompactionStrategy {
    mode: ContextPressureMode,
    policy: ContextCompactionPolicy,
}

pub(in super::super) fn budget_and_compact_summary(
    base: Value,
    state: &EngineRunnerState,
    token_budget: usize,
) -> Value {
    let adaptive_budget = derive_adaptive_planner_budget(state, token_budget);
    let strategy = resolve_context_compaction_strategy(adaptive_budget);
    let mut budgeted = apply_planner_context_budget(base, adaptive_budget, strategy);
    let overflow = budgeted
        .pointer("/context_budget/pack_overflow_reason")
        .and_then(Value::as_str)
        .is_some();
    if overflow {
        budgeted = compact_json_with_options(&budgeted, &strategy.policy.final_compact_options);
    }
    if let Some(context_budget) = budgeted
        .pointer_mut("/context_budget")
        .and_then(Value::as_object_mut)
    {
        context_budget.insert("final_compact_applied".to_string(), Value::Bool(overflow));
    }
    refresh_context_budget_estimates(&mut budgeted);
    budgeted
}

fn derive_adaptive_planner_budget(
    state: &EngineRunnerState,
    base_token_limit: usize,
) -> AdaptivePlannerBudget {
    let usage = state.runtime.pointer("/agent/llm_usage");
    let window_input_tokens = usage
        .and_then(|value| value.get("context_window_input_tokens"))
        .and_then(Value::as_u64);
    let remaining_tokens = usage
        .and_then(|value| value.get("context_remaining_tokens"))
        .and_then(Value::as_u64);
    let soft_limit_tokens = usage
        .and_then(|value| value.get("context_soft_limit_tokens"))
        .and_then(Value::as_u64);
    let usage_ratio_from_window_bps = match (window_input_tokens, soft_limit_tokens) {
        (Some(window_input), Some(soft_limit)) if soft_limit > 0 => {
            Some(window_input.saturating_mul(10_000) / soft_limit)
        }
        _ => None,
    };
    let usage_ratio_from_remaining_bps =
        usage_ratio_bps_from_remaining_tokens(remaining_tokens, soft_limit_tokens);
    let usage_ratio_bps = match (usage_ratio_from_window_bps, usage_ratio_from_remaining_bps) {
        (Some(window_ratio), Some(remaining_ratio)) => Some(window_ratio.max(remaining_ratio)),
        (Some(window_ratio), None) => Some(window_ratio),
        (None, Some(remaining_ratio)) => Some(remaining_ratio),
        (None, None) => None,
    };

    let (effective, _) = ToolMemoryBudgetPolicy::derive_adaptive_effective_token_limit(
        base_token_limit,
        usage_ratio_bps,
    );

    AdaptivePlannerBudget {
        effective_token_limit: effective,
        remaining_tokens,
        usage_ratio_bps,
    }
}

fn usage_ratio_bps_from_remaining_tokens(
    remaining_tokens: Option<u64>,
    soft_limit_tokens: Option<u64>,
) -> Option<u64> {
    match (remaining_tokens, soft_limit_tokens) {
        (Some(remaining), Some(soft_limit)) if soft_limit > 0 => Some(
            soft_limit
                .saturating_sub(remaining.min(soft_limit))
                .saturating_mul(10_000)
                / soft_limit,
        ),
        _ => None,
    }
}

fn resolve_context_compaction_strategy(budget: AdaptivePlannerBudget) -> ContextCompactionStrategy {
    let pressure_mode = ToolMemoryBudgetPolicy::derive_context_pressure_mode(
        budget.usage_ratio_bps,
        budget.remaining_tokens,
    );
    ContextCompactionStrategy {
        mode: pressure_mode,
        policy: ToolMemoryBudgetPolicy::context_compaction_policy(pressure_mode),
    }
}

fn apply_planner_context_budget(
    mut base: Value,
    budget: AdaptivePlannerBudget,
    strategy: ContextCompactionStrategy,
) -> Value {
    let mut pack_trace = PackTrace::default();
    let pack = pack_blocks(
        &mut base,
        budget.effective_token_limit.max(1) as u64,
        strategy,
        &mut pack_trace,
    );

    if let Some(object) = base.as_object_mut() {
        object.insert(
            "context_budget".to_string(),
            json!({
                "pressure_mode": strategy.mode.as_str(),
                "pack_overflow_reason": pack.overflow_reason,
                "pack_diagnostics": {
                    "packed_blocks_total": pack.diagnostics.packed_blocks_total,
                    "packed_blocks_included": pack.diagnostics.packed_blocks_included,
                    "packed_blocks_evicted": pack.diagnostics.packed_blocks_evicted,
                    "compressed_blocks_total": pack.diagnostics.compressed_blocks_total,
                    "compressed_by_reason": pack.diagnostics.compressed_by_reason,
                    "evicted_by_reason": pack.diagnostics.evicted_by_reason,
                },
                "pack_trace": pack_trace
                    .decisions
                    .iter()
                    .map(|decision| {
                        json!({
                            "block_id": decision.block_id,
                            "action": decision.action.as_str(),
                            "reason": decision.reason,
                            "before_level": decision.before_level.map(|level| level.as_str()),
                            "after_level": decision.after_level.map(|level| level.as_str()),
                        })
                    })
                    .collect::<Vec<_>>(),
            }),
        );
    }

    let _ = pack;
    base
}

fn refresh_context_budget_estimates(summary: &mut Value) {
    let _ = summary;
}

#[derive(Debug, Clone)]
struct PackBlocksResult {
    overflow_reason: Option<&'static str>,
    diagnostics: PackDiagnostics,
}

const MUST_KEEP_CONTEXT_POINTERS: [(&str, &str); 3] = [
    ("todo_state", "/todo_state"),
    ("input_registry.known_refs", "/input_registry/known_refs"),
    ("previous_error", "/previous_error"),
];

fn pack_blocks(
    summary: &mut Value,
    token_budget: u64,
    strategy: ContextCompactionStrategy,
    trace: &mut PackTrace,
) -> PackBlocksResult {
    // Build a small set of optional blocks and let a single loop decide compress/drop.
    let mut blocks = ContextPackBlockId::optional_pack_blocks()
        .iter()
        .copied()
        .map(PackBlock::new)
        .collect::<Vec<_>>();

    // Seed full candidates from the current summary projection.
    for block in &mut blocks {
        block.seed_from_summary(summary);
        block.prepare_candidates(strategy.mode);
    }

    // Apply initial full selections.
    for block in &blocks {
        block.apply_selected(summary);
    }

    let baseline_tokens = estimate_tokens_json(summary);
    if baseline_tokens <= token_budget {
        trace.push(PackDecision {
            block_id: "state_summary".to_string(),
            action: PackAction::Keep,
            reason: "within_budget_full",
            before_level: None,
            after_level: None,
        });
        return PackBlocksResult {
            overflow_reason: None,
            diagnostics: pack_diagnostics_from_blocks(&blocks, trace),
        };
    }

    // Single convergence loop.
    for _ in 0..64 {
        let current_tokens = estimate_tokens_json(summary);
        if current_tokens <= token_budget {
            return PackBlocksResult {
                overflow_reason: None,
                diagnostics: pack_diagnostics_from_blocks(&blocks, trace),
            };
        }

        // 1) Under pressure, compress low + stale blocks first.
        if let Some((index, next_level, reason)) = select_next_compress(
            &blocks,
            &[ContextBlockPriority::Low, ContextBlockPriority::Stale],
        ) {
            let block = &mut blocks[index];
            let before = block.selected;
            block.selected = next_level;
            block.apply_selected(summary);
            trace.push(PackDecision {
                block_id: block.id.as_str().to_string(),
                action: PackAction::Compress,
                reason,
                before_level: Some(before),
                after_level: Some(next_level),
            });
            continue;
        }

        // 2) If still over, drop stale then low blocks.
        if let Some((index, reason)) = select_next_drop(
            &blocks,
            &[ContextBlockPriority::Stale, ContextBlockPriority::Low],
        ) {
            let block = &mut blocks[index];
            let before = block.selected;
            block.selected = ContextCompressLevel::Skeleton;
            block.dropped = true;
            block.apply_selected(summary);
            trace.push(PackDecision {
                block_id: block.id.as_str().to_string(),
                action: PackAction::Drop,
                reason,
                before_level: Some(before),
                after_level: Some(ContextCompressLevel::Skeleton),
            });
            continue;
        }

        // 3) If still over, start compressing medium-priority blocks.
        if let Some((index, next_level, reason)) =
            select_next_compress(&blocks, &[ContextBlockPriority::Medium])
        {
            let block = &mut blocks[index];
            let before = block.selected;
            block.selected = next_level;
            block.apply_selected(summary);
            trace.push(PackDecision {
                block_id: block.id.as_str().to_string(),
                action: PackAction::Compress,
                reason,
                before_level: Some(before),
                after_level: Some(next_level),
            });
            continue;
        }

        // 4) If still over, drop medium blocks after they've reached skeleton.
        if let Some((index, reason)) = select_next_drop(&blocks, &[ContextBlockPriority::Medium]) {
            let block = &mut blocks[index];
            let before = block.selected;
            block.selected = ContextCompressLevel::Skeleton;
            block.dropped = true;
            block.apply_selected(summary);
            trace.push(PackDecision {
                block_id: block.id.as_str().to_string(),
                action: PackAction::Drop,
                reason,
                before_level: Some(before),
                after_level: Some(ContextCompressLevel::Skeleton),
            });
            continue;
        }

        // 5) No more evictable/compressible options.
        let must_keep_only = blocks
            .iter()
            .all(|block| block.dropped || block.selected == ContextCompressLevel::Skeleton)
            && MUST_KEEP_CONTEXT_POINTERS
                .iter()
                .any(|(_, path)| pointer_has_payload(summary, path));
        trace.push(PackDecision {
            block_id: "state_summary".to_string(),
            action: PackAction::Keep,
            reason: if must_keep_only {
                "must_keep_only_exceeds_budget"
            } else {
                "budget_exceeded_no_further_actions"
            },
            before_level: None,
            after_level: None,
        });
        return PackBlocksResult {
            overflow_reason: Some(if must_keep_only {
                "must_keep_only_exceeds_budget"
            } else {
                "budget_exceeded_no_further_actions"
            }),
            diagnostics: pack_diagnostics_from_blocks(&blocks, trace),
        };
    }

    trace.push(PackDecision {
        block_id: "state_summary".to_string(),
        action: PackAction::Keep,
        reason: "budget_exceeded_pack_loop_limit",
        before_level: None,
        after_level: None,
    });
    PackBlocksResult {
        overflow_reason: Some("budget_exceeded_pack_loop_limit"),
        diagnostics: pack_diagnostics_from_blocks(&blocks, trace),
    }
}

fn pack_diagnostics_from_blocks(blocks: &[PackBlock], trace: &PackTrace) -> PackDiagnostics {
    let mut diagnostics = PackDiagnostics::default();
    diagnostics.packed_blocks_total = blocks.len() as u64;
    diagnostics.packed_blocks_included =
        blocks.iter().filter(|block| !block.dropped).count() as u64;
    diagnostics.packed_blocks_evicted = blocks.iter().filter(|block| block.dropped).count() as u64;

    for decision in &trace.decisions {
        diagnostics.observe_decision(decision);
    }
    diagnostics
}

#[derive(Debug, Clone)]
struct PackBlock {
    id: ContextPackBlockId,
    selected: ContextCompressLevel,
    dropped: bool,
    full: Value,
    summary: Option<Value>,
    skeleton: Value,
}

impl PackBlock {
    fn new(id: ContextPackBlockId) -> Self {
        Self {
            id,
            selected: ContextCompressLevel::Full,
            dropped: false,
            full: Value::Null,
            summary: None,
            skeleton: Value::Null,
        }
    }

    fn seed_from_summary(&mut self, summary: &Value) {
        self.full = summary
            .pointer(self.id.path())
            .cloned()
            .unwrap_or(Value::Null);
    }

    fn prepare_candidates(&mut self, mode: ContextPressureMode) {
        let recipe = ToolMemoryBudgetPolicy::context_pack_block_recipe(self.id, mode);
        self.summary = recipe
            .summary_compact_options
            .map(|options| compact_json_with_options(&self.full, &options))
            .or_else(|| Some(self.full.clone()));
        self.selected = recipe.preferred_level;

        match self.id {
            ContextPackBlockId::ToolMemoryProjection => {
                self.skeleton = Value::Null;
            }
            ContextPackBlockId::InputStoreFacts => {
                self.skeleton = minimal_input_store_facts(&self.full);
            }
            ContextPackBlockId::PreviousErrorLastFailedFinalize => {
                let mut skeleton = self.full.clone();
                let _ = set_pointer_value(&mut skeleton, "/assistant_content", Value::Null);
                self.skeleton = skeleton;
            }
            ContextPackBlockId::CapabilityViewProtocols => {
                self.skeleton = Value::Array(vec![]);
            }
            ContextPackBlockId::InputSlotsCanonicalRefs => {
                self.skeleton = Value::Null;
            }
        }
    }

    fn has_next_compress_level(&self) -> Option<ContextCompressLevel> {
        match self.selected {
            ContextCompressLevel::Full => {
                if self.summary.is_some() {
                    Some(ContextCompressLevel::Summary)
                } else {
                    Some(ContextCompressLevel::Skeleton)
                }
            }
            ContextCompressLevel::Summary => Some(ContextCompressLevel::Skeleton),
            ContextCompressLevel::Skeleton => None,
        }
    }

    fn apply_selected(&self, summary: &mut Value) {
        let value = match self.selected {
            ContextCompressLevel::Full => self.full.clone(),
            ContextCompressLevel::Summary => {
                self.summary.clone().unwrap_or_else(|| self.full.clone())
            }
            ContextCompressLevel::Skeleton => self.skeleton.clone(),
        };
        let _ = set_pointer_value(summary, self.id.path(), value);
        if self.id == ContextPackBlockId::InputStoreFacts {
            reconcile_input_store_meta(summary);
        }
        if self.id == ContextPackBlockId::CapabilityViewProtocols
            && self.selected == ContextCompressLevel::Skeleton
        {
            // Keep counts coherent when protocols dropped.
            let topic_count = summary
                .pointer("/capability_view/topics")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            if let Some(slot) = summary.pointer_mut("/capability_view/counts") {
                *slot = json!({
                    "protocols": 0,
                    "actions": 0,
                    "queries": 0,
                    "topics": topic_count,
                });
            }
        }
    }
}

fn minimal_input_store_facts(full: &Value) -> Value {
    let mut out = serde_json::Map::<String, Value>::new();
    let Some(facts) = full.as_object() else {
        return Value::Object(out);
    };
    for key in ["owner", "wallet.default"] {
        if let Some(value) = facts.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(out)
}

fn select_next_compress(
    blocks: &[PackBlock],
    priorities: &[ContextBlockPriority],
) -> Option<(usize, ContextCompressLevel, &'static str)> {
    for &priority in priorities {
        for (index, block) in blocks.iter().enumerate() {
            if block.id.default_priority() != priority {
                continue;
            }
            let Some(next) = block.has_next_compress_level() else {
                continue;
            };
            if matches!(next, ContextCompressLevel::Summary) && block.summary.is_none() {
                continue;
            }
            if block.selected == ContextCompressLevel::Skeleton {
                continue;
            }
            return Some((index, next, "pack_compress"));
        }
    }
    None
}

fn select_next_drop(
    blocks: &[PackBlock],
    priorities: &[ContextBlockPriority],
) -> Option<(usize, &'static str)> {
    for &priority in priorities {
        for (index, block) in blocks.iter().enumerate() {
            if block.id.default_priority() != priority {
                continue;
            }
            if !block.id.is_evictable() || block.dropped {
                continue;
            }
            // Only drop once we've reached skeleton (or can't compress further).
            if block.selected != ContextCompressLevel::Skeleton {
                continue;
            }
            return Some((index, "pack_drop"));
        }
    }
    None
}

fn reconcile_input_store_meta(summary: &mut Value) {
    let Some(facts_object) = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object)
    else {
        return;
    };
    let fact_keys = facts_object
        .keys()
        .filter(|key| !key.starts_with('_'))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let Some(meta_object) = summary
        .pointer_mut("/input_store/meta")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    meta_object.retain(|key, _| fact_keys.contains(key));
}

fn pointer_has_payload(summary: &Value, path: &str) -> bool {
    match summary.pointer(path) {
        Some(Value::Null) | None => false,
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

fn set_pointer_value(target: &mut Value, path: &str, value: Value) -> bool {
    let Some(slot) = target.pointer_mut(path) else {
        return false;
    };
    *slot = value;
    true
}

pub(in super::super) fn estimate_tokens_json(value: &Value) -> u64 {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    let chars = encoded.chars().count();
    chars
        .saturating_add(CHARS_PER_TOKEN_ESTIMATE.saturating_sub(1))
        .checked_div(CHARS_PER_TOKEN_ESTIMATE)
        .unwrap_or(0) as u64
}
