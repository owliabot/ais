use super::intent_segmented::PlannerRoundPhase;
use super::reference_inventory::ReferenceInventory;
use super::state_summary::StateSummary;
use super::tools::phase_policy::phase_name;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub(super) struct PlannerDiagnosticsTracker {
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
    reusable_inventory_total_refs: u64,
    reusable_inventory_reusable_refs: u64,
    reusable_inventory_fresh_volatile_refs: u64,
    reusable_inventory_stale_volatile_refs: u64,
    redundant_query_rejections_total: u64,
    redundant_query_steps_total: u64,
    tool_exec_count_by_tool: BTreeMap<String, u64>,
    tool_exec_error_by_tool: BTreeMap<String, u64>,
    tool_exec_latency_sum_ms_by_tool: BTreeMap<String, u64>,
    tool_exec_latency_max_ms_by_tool: BTreeMap<String, u64>,
    seen_tool_call_keys: BTreeSet<String>,
}

impl PlannerDiagnosticsTracker {
    pub(super) fn observe_phase_round(&mut self, phase: PlannerRoundPhase) {
        let key = phase_name(phase).to_string();
        let entry = self.phase_round_count.entry(key).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    pub(super) fn observe_tool_call(
        &mut self,
        tool_name: &str,
        dedupe_key: Option<String>,
    ) -> bool {
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

    pub(super) fn observe_tool_result(&mut self, tool_name: &str, cached: bool) {
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

    pub(super) fn observe_empty_search_streak(&mut self, streak: u64) {
        if streak > self.empty_search_streak_max {
            self.empty_search_streak_max = streak;
        }
    }

    pub(super) fn observe_finalize_schema_repair_attempt(&mut self, sub_reason_code: &str) {
        self.finalize_schema_repair_attempts_total =
            self.finalize_schema_repair_attempts_total.saturating_add(1);
        let entry = self
            .finalize_schema_repair_by_sub_reason
            .entry(sub_reason_code.to_string())
            .or_insert(0);
        *entry = entry.saturating_add(1);
    }

    pub(super) fn observe_tool_memory_projection(
        &mut self,
        budget_tokens: usize,
        estimated_tokens: Option<u64>,
    ) {
        self.memory_projection_budget_tokens = u64::try_from(budget_tokens).unwrap_or(u64::MAX);
        self.memory_projection_estimated_tokens = estimated_tokens.unwrap_or(0);
    }

    pub(super) fn observe_reusable_inventory_summary(
        &mut self,
        typed_summary: Option<&StateSummary>,
        state_summary: Option<&Value>,
    ) {
        let summary_projection = typed_summary
            .map(|summary| {
                ReferenceInventory::build_typed(Some(summary)).to_reusable_outputs_projection()
            })
            .unwrap_or_else(|| {
                ReferenceInventory::build(state_summary).to_reusable_outputs_projection()
            });
        let summary = summary_projection
            .as_ref()
            .and_then(|value| value.pointer("/summary"))
            .or_else(|| state_summary.and_then(|value| value.pointer("/reusable_outputs/summary")));
        let Some(summary) = summary else {
            return;
        };
        self.reusable_inventory_total_refs = self.reusable_inventory_total_refs.max(
            summary
                .get("total_refs")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        self.reusable_inventory_reusable_refs = self.reusable_inventory_reusable_refs.max(
            summary
                .get("reusable_refs")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        self.reusable_inventory_fresh_volatile_refs =
            self.reusable_inventory_fresh_volatile_refs.max(
                summary
                    .get("fresh_volatile_refs")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
        self.reusable_inventory_stale_volatile_refs =
            self.reusable_inventory_stale_volatile_refs.max(
                summary
                    .get("stale_volatile_refs")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
    }

    pub(super) fn observe_check_segment_redundant_query_rejections(&mut self, payload_text: &str) {
        let Ok(value) = serde_json::from_str::<Value>(payload_text) else {
            return;
        };
        let Some(issues) = value.pointer("/issues").and_then(Value::as_array) else {
            return;
        };
        let redundant_count = issues
            .iter()
            .filter(|issue| {
                issue.get("reason_code").and_then(Value::as_str) == Some("redundant_query_step")
            })
            .count() as u64;
        if redundant_count == 0 {
            return;
        }
        self.redundant_query_rejections_total =
            self.redundant_query_rejections_total.saturating_add(1);
        self.redundant_query_steps_total = self
            .redundant_query_steps_total
            .saturating_add(redundant_count);
    }

    pub(super) fn observe_finalize_schema_repair_exhausted(&mut self) {
        self.finalize_schema_repair_exhausted_total = self
            .finalize_schema_repair_exhausted_total
            .saturating_add(1);
    }

    pub(super) fn observe_no_toolcall_retry(&mut self) {
        self.no_toolcall_retries_total = self.no_toolcall_retries_total.saturating_add(1);
    }

    pub(super) fn observe_no_toolcall_retry_exhausted(&mut self) {
        self.no_toolcall_retries_exhausted_total =
            self.no_toolcall_retries_exhausted_total.saturating_add(1);
    }

    pub(super) fn observe_parallel_batch(&mut self, calls: u64) {
        self.parallel_batches_total = self.parallel_batches_total.saturating_add(1);
        self.parallel_calls_total = self.parallel_calls_total.saturating_add(calls);
    }

    pub(super) fn observe_parallel_partial_success(&mut self) {
        self.parallel_partial_success_total = self.parallel_partial_success_total.saturating_add(1);
    }

    pub(super) fn observe_tool_exec_end(
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

    pub(super) fn observe_tool_exec_retry(&mut self, exhausted: bool) {
        if exhausted {
            self.tool_exec_repair_exhausted = self.tool_exec_repair_exhausted.saturating_add(1);
        } else {
            self.tool_exec_repair_retry = self.tool_exec_repair_retry.saturating_add(1);
        }
    }

    pub(super) fn duplicate_ratio_bps(&self) -> u64 {
        if self.total_tool_calls == 0 {
            return 0;
        }
        self.duplicate_tool_calls.saturating_mul(10_000) / self.total_tool_calls
    }

    pub(super) fn empty_search_streak_max(&self) -> u64 {
        self.empty_search_streak_max
    }

    fn discovery_ratio_bps(&self) -> u64 {
        if self.total_tool_calls == 0 {
            return 0;
        }
        let discovery_calls = ["catalog.discover", "get_candidate_detail", "guide.get"]
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

    pub(super) fn to_value(&self) -> Value {
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
            },
            "reusable_inventory": {
                "total_refs": self.reusable_inventory_total_refs,
                "reusable_refs": self.reusable_inventory_reusable_refs,
                "fresh_volatile_refs": self.reusable_inventory_fresh_volatile_refs,
                "stale_volatile_refs": self.reusable_inventory_stale_volatile_refs,
            },
            "redundant_query_rejections": {
                "total": self.redundant_query_rejections_total,
                "steps_total": self.redundant_query_steps_total,
            }
        })
    }
}
