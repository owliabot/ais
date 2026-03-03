use super::super::budget::JsonBudgetOptions;
use super::packing::{ContextCompressLevel, ContextPackBlockId};
use serde_json::Value;

/// Single-source scaffold for tool-memory and context budget policy.
///
/// Wave-CB-A keeps behavior unchanged by mirroring existing constants and helper
/// formulas in a dedicated module. Later waves will migrate callers to this API.

#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolMemoryBudgetPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanningMemoryStoreBudget {
    pub(crate) max_entries: usize,
    pub(crate) max_entry_chars: usize,
    pub(crate) max_total_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolMemoryProjectionCaps {
    pub(crate) max_list_inventory_entries: usize,
    pub(crate) max_catalog_entries: usize,
    pub(crate) max_detail_entries: usize,
    pub(crate) max_guide_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolDispatchCompactProfile {
    Tight,
    Balanced,
    Relaxed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolDispatchKind {
    CandidateDetail,
    MissingFacts,
    GuideSchemaFull,
    GuideSchemaDigest,
    GuideTopic,
    CheckSegment,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextCompactionPolicy {
    pub(crate) final_compact_options: JsonBudgetOptions,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextPackBlockRecipe {
    pub(crate) summary_compact_options: Option<JsonBudgetOptions>,
    pub(crate) preferred_level: ContextCompressLevel,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextStrategyTable {
    pub(crate) planner_context_default_token_budget: usize,
    pub(crate) adaptive_relaxed_max_multiplier: usize,
    pub(crate) adaptive_medium_numerator: usize,
    pub(crate) adaptive_medium_denominator: usize,
    pub(crate) adaptive_critical_numerator: usize,
    pub(crate) adaptive_critical_denominator: usize,
    pub(crate) context_usage_light_threshold_bps: u64,
    pub(crate) context_usage_medium_threshold_bps: u64,
    pub(crate) context_usage_critical_threshold_bps: u64,
    pub(crate) context_pressure_tight_remaining_tokens: u64,
    pub(crate) context_pressure_critical_remaining_tokens: u64,
    pub(crate) tool_memory_projection_min_tokens: usize,
    pub(crate) tool_memory_projection_default_tokens: usize,
    pub(crate) tool_memory_projection_max_tokens: usize,
    pub(crate) tool_memory_projection_abs_min_tokens: usize,
    pub(crate) tool_memory_projection_abs_max_tokens: usize,
    pub(crate) tool_memory_projection_soft_limit_min_ratio_bps: u64,
    pub(crate) tool_memory_projection_soft_limit_max_ratio_bps: u64,
    pub(crate) tool_memory_projection_tight_threshold_bps: u64,
    pub(crate) tool_memory_projection_relaxed_threshold_bps: u64,
    pub(crate) tool_memory_remaining_abs_min: u64,
    pub(crate) tool_memory_remaining_abs_max: u64,
    pub(crate) tool_memory_max_list_inventory_entries: usize,
    pub(crate) tool_memory_max_catalog_entries: usize,
    pub(crate) tool_memory_max_detail_entries: usize,
    pub(crate) tool_memory_max_guide_entries: usize,
    pub(crate) planning_memory_store_default_max_entries: usize,
    pub(crate) planning_memory_store_default_max_entry_chars: usize,
    pub(crate) planning_memory_store_default_max_total_chars: usize,
}

const CONTEXT_STRATEGY_TABLE: ContextStrategyTable = ContextStrategyTable {
    planner_context_default_token_budget: 6_000,
    adaptive_relaxed_max_multiplier: 3,
    adaptive_medium_numerator: 17,
    adaptive_medium_denominator: 20,
    adaptive_critical_numerator: 3,
    adaptive_critical_denominator: 5,
    context_usage_light_threshold_bps: 7_000,
    context_usage_medium_threshold_bps: 8_500,
    context_usage_critical_threshold_bps: 9_200,
    context_pressure_tight_remaining_tokens: 8_000,
    context_pressure_critical_remaining_tokens: 3_000,
    tool_memory_projection_min_tokens: 1200,
    tool_memory_projection_default_tokens: 2400,
    tool_memory_projection_max_tokens: 6000,
    tool_memory_projection_abs_min_tokens: 1200,
    tool_memory_projection_abs_max_tokens: 64_000,
    tool_memory_projection_soft_limit_min_ratio_bps: 2_000,
    tool_memory_projection_soft_limit_max_ratio_bps: 4_000,
    tool_memory_projection_tight_threshold_bps: 2000,
    tool_memory_projection_relaxed_threshold_bps: 6000,
    tool_memory_remaining_abs_min: 4_000,
    tool_memory_remaining_abs_max: 24_000,
    tool_memory_max_list_inventory_entries: 2,
    tool_memory_max_catalog_entries: 6,
    tool_memory_max_detail_entries: 6,
    tool_memory_max_guide_entries: 4,
    planning_memory_store_default_max_entries: 48,
    planning_memory_store_default_max_entry_chars: 8_000,
    planning_memory_store_default_max_total_chars: 120_000,
};

impl ToolMemoryBudgetPolicy {
    // Legacy aliases while callsites are migrated to strategy-table field access.
    pub(crate) const PLANNER_CONTEXT_DEFAULT_TOKEN_BUDGET: usize =
        CONTEXT_STRATEGY_TABLE.planner_context_default_token_budget;
    pub(crate) const ADAPTIVE_RELAXED_MAX_MULTIPLIER: usize =
        CONTEXT_STRATEGY_TABLE.adaptive_relaxed_max_multiplier;
    pub(crate) const ADAPTIVE_MEDIUM_NUMERATOR: usize =
        CONTEXT_STRATEGY_TABLE.adaptive_medium_numerator;
    pub(crate) const ADAPTIVE_MEDIUM_DENOMINATOR: usize =
        CONTEXT_STRATEGY_TABLE.adaptive_medium_denominator;
    pub(crate) const ADAPTIVE_CRITICAL_NUMERATOR: usize =
        CONTEXT_STRATEGY_TABLE.adaptive_critical_numerator;
    pub(crate) const ADAPTIVE_CRITICAL_DENOMINATOR: usize =
        CONTEXT_STRATEGY_TABLE.adaptive_critical_denominator;
    pub(crate) const CONTEXT_USAGE_LIGHT_THRESHOLD_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.context_usage_light_threshold_bps;
    pub(crate) const CONTEXT_USAGE_MEDIUM_THRESHOLD_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.context_usage_medium_threshold_bps;
    pub(crate) const CONTEXT_USAGE_CRITICAL_THRESHOLD_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.context_usage_critical_threshold_bps;
    pub(crate) const CONTEXT_PRESSURE_TIGHT_REMAINING_TOKENS: u64 =
        CONTEXT_STRATEGY_TABLE.context_pressure_tight_remaining_tokens;
    pub(crate) const CONTEXT_PRESSURE_CRITICAL_REMAINING_TOKENS: u64 =
        CONTEXT_STRATEGY_TABLE.context_pressure_critical_remaining_tokens;
    pub(crate) const TOOL_MEMORY_PROJECTION_MIN_TOKENS: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_min_tokens;
    pub(crate) const TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_default_tokens;
    pub(crate) const TOOL_MEMORY_PROJECTION_MAX_TOKENS: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_max_tokens;
    pub(crate) const TOOL_MEMORY_PROJECTION_ABS_MIN_TOKENS: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_abs_min_tokens;
    pub(crate) const TOOL_MEMORY_PROJECTION_ABS_MAX_TOKENS: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_abs_max_tokens;
    pub(crate) const TOOL_MEMORY_PROJECTION_SOFT_LIMIT_MIN_RATIO_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_soft_limit_min_ratio_bps;
    pub(crate) const TOOL_MEMORY_PROJECTION_SOFT_LIMIT_MAX_RATIO_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_soft_limit_max_ratio_bps;
    pub(crate) const TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_tight_threshold_bps;
    pub(crate) const TOOL_MEMORY_PROJECTION_RELAXED_THRESHOLD_BPS: u64 =
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_relaxed_threshold_bps;
    pub(crate) const TOOL_MEMORY_REMAINING_ABS_MIN: u64 =
        CONTEXT_STRATEGY_TABLE.tool_memory_remaining_abs_min;
    pub(crate) const TOOL_MEMORY_REMAINING_ABS_MAX: u64 =
        CONTEXT_STRATEGY_TABLE.tool_memory_remaining_abs_max;
    pub(crate) const TOOL_MEMORY_MAX_LIST_INVENTORY_ENTRIES: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_max_list_inventory_entries;
    pub(crate) const TOOL_MEMORY_MAX_CATALOG_ENTRIES: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_max_catalog_entries;
    pub(crate) const TOOL_MEMORY_MAX_DETAIL_ENTRIES: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_max_detail_entries;
    pub(crate) const TOOL_MEMORY_MAX_GUIDE_ENTRIES: usize =
        CONTEXT_STRATEGY_TABLE.tool_memory_max_guide_entries;
    pub(crate) const PLANNING_MEMORY_STORE_DEFAULT_MAX_ENTRIES: usize =
        CONTEXT_STRATEGY_TABLE.planning_memory_store_default_max_entries;
    pub(crate) const PLANNING_MEMORY_STORE_DEFAULT_MAX_ENTRY_CHARS: usize =
        CONTEXT_STRATEGY_TABLE.planning_memory_store_default_max_entry_chars;
    pub(crate) const PLANNING_MEMORY_STORE_DEFAULT_MAX_TOTAL_CHARS: usize =
        CONTEXT_STRATEGY_TABLE.planning_memory_store_default_max_total_chars;

    pub(crate) const fn tool_memory_projection_default_tokens() -> usize {
        CONTEXT_STRATEGY_TABLE.tool_memory_projection_default_tokens
    }

    pub(crate) const fn tool_memory_projection_abs_bounds() -> (usize, usize) {
        (
            CONTEXT_STRATEGY_TABLE.tool_memory_projection_abs_min_tokens,
            CONTEXT_STRATEGY_TABLE.tool_memory_projection_abs_max_tokens,
        )
    }

    pub(crate) fn derive_tool_memory_projection_token_budget(
        planner_usage: Option<&Value>,
        runtime_usage: Option<&Value>,
    ) -> usize {
        let remaining_tokens = usage_field_u64(planner_usage, "context_remaining_tokens")
            .or_else(|| usage_field_u64(runtime_usage, "context_remaining_tokens"));
        let soft_limit_tokens = usage_field_u64(planner_usage, "context_soft_limit_tokens")
            .or_else(|| usage_field_u64(runtime_usage, "context_soft_limit_tokens"));

        if let (Some(remaining), Some(soft_limit)) = (remaining_tokens, soft_limit_tokens) {
            if soft_limit > 0 {
                let ratio_bps = remaining.saturating_mul(10_000) / soft_limit;
                let (min_budget, max_budget) = budget_bounds_from_soft_limit(soft_limit);
                return budget_from_ratio_bps(ratio_bps, min_budget, max_budget);
            }
        }
        if let Some(remaining) = remaining_tokens {
            return budget_from_remaining_tokens(remaining);
        }
        Self::TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS
    }

    pub(crate) fn derive_context_pressure_mode(
        usage_ratio_bps: Option<u64>,
        remaining_tokens: Option<u64>,
    ) -> ContextPressureMode {
        if usage_ratio_bps.is_some_and(|ratio| ratio >= Self::CONTEXT_USAGE_CRITICAL_THRESHOLD_BPS)
            || remaining_tokens.is_some_and(|remaining| {
                remaining <= Self::CONTEXT_PRESSURE_CRITICAL_REMAINING_TOKENS
            })
        {
            return ContextPressureMode::Critical;
        }
        if usage_ratio_bps.is_some_and(|ratio| ratio >= Self::CONTEXT_USAGE_MEDIUM_THRESHOLD_BPS)
            || remaining_tokens
                .is_some_and(|remaining| remaining <= Self::CONTEXT_PRESSURE_TIGHT_REMAINING_TOKENS)
        {
            return ContextPressureMode::Medium;
        }
        if usage_ratio_bps.is_some_and(|ratio| ratio >= Self::CONTEXT_USAGE_LIGHT_THRESHOLD_BPS) {
            return ContextPressureMode::Light;
        }
        ContextPressureMode::Normal
    }

    pub(crate) fn derive_planning_memory_store_budget(
        planner_usage: Option<&Value>,
        runtime_usage: Option<&Value>,
    ) -> PlanningMemoryStoreBudget {
        let (remaining_tokens, soft_limit_tokens, usage_ratio_bps) =
            usage_inputs(planner_usage, runtime_usage);
        if remaining_tokens.is_none() && usage_ratio_bps.is_none() {
            return PlanningMemoryStoreBudget {
                max_entries: Self::PLANNING_MEMORY_STORE_DEFAULT_MAX_ENTRIES,
                max_entry_chars: Self::PLANNING_MEMORY_STORE_DEFAULT_MAX_ENTRY_CHARS,
                max_total_chars: Self::PLANNING_MEMORY_STORE_DEFAULT_MAX_TOTAL_CHARS,
            };
        }

        let projection_budget =
            Self::derive_tool_memory_projection_token_budget(planner_usage, runtime_usage);
        let (projection_min, projection_max) = soft_limit_tokens
            .map(budget_bounds_from_soft_limit)
            .unwrap_or((
                Self::TOOL_MEMORY_PROJECTION_MIN_TOKENS,
                Self::TOOL_MEMORY_PROJECTION_MAX_TOKENS,
            ));
        let projection_span = projection_max.saturating_sub(projection_min).max(1);
        let projection_progress = projection_budget
            .saturating_sub(projection_min)
            .min(projection_span);
        let progress_bps = u64::try_from(projection_progress)
            .unwrap_or(0)
            .saturating_mul(10_000)
            / u64::try_from(projection_span).unwrap_or(1);

        let mut max_entries = interpolate_usize(16, 72, progress_bps);
        let mut max_entry_chars = interpolate_usize(3_000, 10_000, progress_bps);
        let mut max_total_chars = interpolate_usize(40_000, 180_000, progress_bps);

        let pressure_mode = Self::derive_context_pressure_mode(usage_ratio_bps, remaining_tokens);
        match pressure_mode {
            ContextPressureMode::Critical => {
                max_entries = max_entries.min(16);
                max_entry_chars = max_entry_chars.min(3_000);
                max_total_chars = max_total_chars.min(40_000);
            }
            ContextPressureMode::Medium => {
                max_entries = max_entries.min(32);
                max_entry_chars = max_entry_chars.min(6_000);
                max_total_chars = max_total_chars.min(80_000);
            }
            ContextPressureMode::Light => {
                max_entries = max_entries.min(48);
                max_entry_chars = max_entry_chars.min(8_000);
                max_total_chars = max_total_chars.min(120_000);
            }
            ContextPressureMode::Normal => {}
        }

        PlanningMemoryStoreBudget {
            max_entries: max_entries.max(1),
            max_entry_chars: max_entry_chars.max(1),
            max_total_chars: max_total_chars.max(max_entry_chars.max(1)),
        }
    }

    pub(crate) fn derive_tool_memory_projection_caps(
        projection_budget_tokens: usize,
    ) -> ToolMemoryProjectionCaps {
        let normalized = projection_budget_tokens.clamp(
            Self::TOOL_MEMORY_PROJECTION_ABS_MIN_TOKENS,
            Self::TOOL_MEMORY_PROJECTION_ABS_MAX_TOKENS,
        );
        let progress_bps = normalized
            .saturating_sub(Self::TOOL_MEMORY_PROJECTION_ABS_MIN_TOKENS)
            .saturating_mul(10_000)
            / Self::TOOL_MEMORY_PROJECTION_ABS_MAX_TOKENS
                .saturating_sub(Self::TOOL_MEMORY_PROJECTION_ABS_MIN_TOKENS)
                .max(1);
        ToolMemoryProjectionCaps {
            max_list_inventory_entries: interpolate_usize(
                Self::TOOL_MEMORY_MAX_LIST_INVENTORY_ENTRIES,
                8,
                progress_bps as u64,
            ),
            max_catalog_entries: interpolate_usize(
                Self::TOOL_MEMORY_MAX_CATALOG_ENTRIES,
                20,
                progress_bps as u64,
            ),
            max_detail_entries: interpolate_usize(
                Self::TOOL_MEMORY_MAX_DETAIL_ENTRIES,
                20,
                progress_bps as u64,
            ),
            max_guide_entries: interpolate_usize(
                Self::TOOL_MEMORY_MAX_GUIDE_ENTRIES,
                10,
                progress_bps as u64,
            ),
        }
    }

    pub(crate) fn derive_tool_dispatch_compact_profile(
        projection_budget_tokens: usize,
    ) -> ToolDispatchCompactProfile {
        if projection_budget_tokens >= 20_000 {
            return ToolDispatchCompactProfile::Relaxed;
        }
        if projection_budget_tokens >= 6_000 {
            return ToolDispatchCompactProfile::Balanced;
        }
        ToolDispatchCompactProfile::Tight
    }

    pub(crate) fn derive_global_compress_level(
        pressure_mode: ContextPressureMode,
    ) -> ContextCompressLevel {
        match pressure_mode {
            ContextPressureMode::Critical => ContextCompressLevel::Skeleton,
            ContextPressureMode::Medium => ContextCompressLevel::Summary,
            ContextPressureMode::Light | ContextPressureMode::Normal => ContextCompressLevel::Full,
        }
    }

    pub(crate) fn derive_tool_dispatch_compact_profile_from_compress_level(
        level: ContextCompressLevel,
    ) -> ToolDispatchCompactProfile {
        match level {
            ContextCompressLevel::Full => ToolDispatchCompactProfile::Relaxed,
            ContextCompressLevel::Summary => ToolDispatchCompactProfile::Balanced,
            ContextCompressLevel::Skeleton => ToolDispatchCompactProfile::Tight,
        }
    }

    pub(crate) fn tool_dispatch_options(
        kind: ToolDispatchKind,
        profile: ToolDispatchCompactProfile,
    ) -> JsonBudgetOptions {
        match (kind, profile) {
            (ToolDispatchKind::CandidateDetail, ToolDispatchCompactProfile::Tight) => {
                JsonBudgetOptions {
                    max_depth: 6,
                    max_object_entries: 48,
                    max_array_items: 24,
                    max_string_chars: 1200,
                }
            }
            (ToolDispatchKind::CandidateDetail, ToolDispatchCompactProfile::Balanced) => {
                JsonBudgetOptions {
                    max_depth: 8,
                    max_object_entries: 96,
                    max_array_items: 48,
                    max_string_chars: 2400,
                }
            }
            (ToolDispatchKind::CandidateDetail, ToolDispatchCompactProfile::Relaxed) => {
                JsonBudgetOptions {
                    max_depth: 10,
                    max_object_entries: 192,
                    max_array_items: 96,
                    max_string_chars: 4000,
                }
            }
            (ToolDispatchKind::MissingFacts, ToolDispatchCompactProfile::Tight) => {
                JsonBudgetOptions {
                    max_depth: 7,
                    max_object_entries: 96,
                    max_array_items: 48,
                    max_string_chars: 1600,
                }
            }
            (ToolDispatchKind::MissingFacts, ToolDispatchCompactProfile::Balanced) => {
                JsonBudgetOptions {
                    max_depth: 8,
                    max_object_entries: 128,
                    max_array_items: 64,
                    max_string_chars: 2400,
                }
            }
            (ToolDispatchKind::MissingFacts, ToolDispatchCompactProfile::Relaxed) => {
                JsonBudgetOptions {
                    max_depth: 10,
                    max_object_entries: 192,
                    max_array_items: 96,
                    max_string_chars: 3600,
                }
            }
            (ToolDispatchKind::GuideSchemaFull, ToolDispatchCompactProfile::Tight) => {
                JsonBudgetOptions {
                    max_depth: 32,
                    max_object_entries: 2048,
                    max_array_items: 512,
                    max_string_chars: 8000,
                }
            }
            (ToolDispatchKind::GuideSchemaFull, ToolDispatchCompactProfile::Balanced) => {
                JsonBudgetOptions {
                    max_depth: 64,
                    max_object_entries: 4096,
                    max_array_items: 1024,
                    max_string_chars: 16_000,
                }
            }
            (ToolDispatchKind::GuideSchemaFull, ToolDispatchCompactProfile::Relaxed) => {
                JsonBudgetOptions {
                    max_depth: 80,
                    max_object_entries: 8192,
                    max_array_items: 2048,
                    max_string_chars: 24_000,
                }
            }
            (ToolDispatchKind::GuideSchemaDigest, ToolDispatchCompactProfile::Tight) => {
                JsonBudgetOptions {
                    max_depth: 8,
                    max_object_entries: 96,
                    max_array_items: 48,
                    max_string_chars: 1200,
                }
            }
            (ToolDispatchKind::GuideSchemaDigest, ToolDispatchCompactProfile::Balanced) => {
                JsonBudgetOptions {
                    max_depth: 10,
                    max_object_entries: 128,
                    max_array_items: 64,
                    max_string_chars: 1600,
                }
            }
            (ToolDispatchKind::GuideSchemaDigest, ToolDispatchCompactProfile::Relaxed) => {
                JsonBudgetOptions {
                    max_depth: 12,
                    max_object_entries: 192,
                    max_array_items: 96,
                    max_string_chars: 2400,
                }
            }
            (
                ToolDispatchKind::GuideTopic | ToolDispatchKind::CheckSegment,
                ToolDispatchCompactProfile::Tight,
            ) => JsonBudgetOptions {
                max_depth: 7,
                max_object_entries: 48,
                max_array_items: 16,
                max_string_chars: 1600,
            },
            (
                ToolDispatchKind::GuideTopic | ToolDispatchKind::CheckSegment,
                ToolDispatchCompactProfile::Balanced,
            ) => JsonBudgetOptions {
                max_depth: 8,
                max_object_entries: 64,
                max_array_items: 24,
                max_string_chars: 2400,
            },
            (
                ToolDispatchKind::GuideTopic | ToolDispatchKind::CheckSegment,
                ToolDispatchCompactProfile::Relaxed,
            ) => JsonBudgetOptions {
                max_depth: 10,
                max_object_entries: 96,
                max_array_items: 48,
                max_string_chars: 3600,
            },
        }
    }

    pub(crate) fn derive_adaptive_effective_token_limit(
        base_token_limit: usize,
        usage_ratio_bps: Option<u64>,
    ) -> (usize, &'static str) {
        let mut effective = base_token_limit.max(1);
        let mut mode = "default";
        if let Some(usage_bps) = usage_ratio_bps {
            if usage_bps < Self::CONTEXT_USAGE_LIGHT_THRESHOLD_BPS {
                let relaxed_cap = base_token_limit
                    .saturating_mul(Self::ADAPTIVE_RELAXED_MAX_MULTIPLIER)
                    .max(base_token_limit);
                let relaxed_target = base_token_limit.saturating_mul(3).saturating_div(2);
                effective = relaxed_target.max(base_token_limit).min(relaxed_cap).max(1);
                mode = "relaxed";
            } else if usage_bps < Self::CONTEXT_USAGE_MEDIUM_THRESHOLD_BPS {
                effective = base_token_limit.max(1);
                mode = "balanced";
            } else if usage_bps < Self::CONTEXT_USAGE_CRITICAL_THRESHOLD_BPS {
                effective = base_token_limit.saturating_mul(Self::ADAPTIVE_MEDIUM_NUMERATOR)
                    / Self::ADAPTIVE_MEDIUM_DENOMINATOR;
                effective = effective.max(1);
                mode = "medium";
            } else {
                effective = base_token_limit.saturating_mul(Self::ADAPTIVE_CRITICAL_NUMERATOR)
                    / Self::ADAPTIVE_CRITICAL_DENOMINATOR;
                effective = effective.max(1);
                mode = "tight";
            }
        }
        (effective, mode)
    }

    pub(crate) fn context_compaction_policy(mode: ContextPressureMode) -> ContextCompactionPolicy {
        match mode {
            ContextPressureMode::Critical => ContextCompactionPolicy {
                final_compact_options: JsonBudgetOptions {
                    max_depth: 8,
                    max_object_entries: 96,
                    max_array_items: 64,
                    max_string_chars: 1200,
                },
            },
            ContextPressureMode::Medium => ContextCompactionPolicy {
                final_compact_options: JsonBudgetOptions {
                    max_depth: 9,
                    max_object_entries: 112,
                    max_array_items: 96,
                    max_string_chars: 2048,
                },
            },
            ContextPressureMode::Light => ContextCompactionPolicy {
                final_compact_options: JsonBudgetOptions {
                    max_depth: 10,
                    max_object_entries: 120,
                    max_array_items: 120,
                    max_string_chars: 3072,
                },
            },
            ContextPressureMode::Normal => ContextCompactionPolicy {
                final_compact_options: JsonBudgetOptions {
                    max_depth: 10,
                    max_object_entries: 128,
                    max_array_items: 128,
                    max_string_chars: 4096,
                },
            },
        }
    }

    pub(crate) fn context_pack_block_recipe(
        block_id: ContextPackBlockId,
        mode: ContextPressureMode,
    ) -> ContextPackBlockRecipe {
        match block_id {
            ContextPackBlockId::ToolMemoryProjection => {
                let summary_compact_options = match mode {
                    ContextPressureMode::Critical => Some(JsonBudgetOptions {
                        max_depth: 5,
                        max_object_entries: 24,
                        max_array_items: 12,
                        max_string_chars: 480,
                    }),
                    ContextPressureMode::Medium => Some(JsonBudgetOptions {
                        max_depth: 6,
                        max_object_entries: 36,
                        max_array_items: 16,
                        max_string_chars: 800,
                    }),
                    ContextPressureMode::Light => Some(JsonBudgetOptions {
                        max_depth: 8,
                        max_object_entries: 96,
                        max_array_items: 48,
                        max_string_chars: 1800,
                    }),
                    ContextPressureMode::Normal => None,
                };
                ContextPackBlockRecipe {
                    summary_compact_options,
                    preferred_level: ContextCompressLevel::Full,
                }
            }
            ContextPackBlockId::InputStoreFacts => {
                let summary_compact_options = match mode {
                    ContextPressureMode::Critical => Some(JsonBudgetOptions {
                        max_depth: 5,
                        max_object_entries: 16,
                        max_array_items: 12,
                        max_string_chars: 480,
                    }),
                    ContextPressureMode::Medium => Some(JsonBudgetOptions {
                        max_depth: 6,
                        max_object_entries: 24,
                        max_array_items: 16,
                        max_string_chars: 720,
                    }),
                    ContextPressureMode::Light => Some(JsonBudgetOptions {
                        max_depth: 8,
                        max_object_entries: 48,
                        max_array_items: 24,
                        max_string_chars: 1200,
                    }),
                    ContextPressureMode::Normal => None,
                };
                ContextPackBlockRecipe {
                    summary_compact_options,
                    preferred_level: if matches!(
                        mode,
                        ContextPressureMode::Critical | ContextPressureMode::Medium
                    ) {
                        ContextCompressLevel::Summary
                    } else {
                        ContextCompressLevel::Full
                    },
                }
            }
            ContextPackBlockId::PreviousErrorLastFailedFinalize => {
                let summary_compact_options = match mode {
                    ContextPressureMode::Critical => Some(JsonBudgetOptions {
                        max_depth: 5,
                        max_object_entries: 32,
                        max_array_items: 12,
                        max_string_chars: 560,
                    }),
                    ContextPressureMode::Medium => Some(JsonBudgetOptions {
                        max_depth: 6,
                        max_object_entries: 48,
                        max_array_items: 20,
                        max_string_chars: 900,
                    }),
                    ContextPressureMode::Light => Some(JsonBudgetOptions {
                        max_depth: 8,
                        max_object_entries: 96,
                        max_array_items: 48,
                        max_string_chars: 2000,
                    }),
                    ContextPressureMode::Normal => None,
                };
                ContextPackBlockRecipe {
                    summary_compact_options,
                    preferred_level: if matches!(mode, ContextPressureMode::Critical) {
                        ContextCompressLevel::Skeleton
                    } else {
                        ContextCompressLevel::Full
                    },
                }
            }
            ContextPackBlockId::CapabilityViewProtocols => ContextPackBlockRecipe {
                summary_compact_options: None,
                preferred_level: if matches!(mode, ContextPressureMode::Critical) {
                    ContextCompressLevel::Skeleton
                } else {
                    ContextCompressLevel::Full
                },
            },
            ContextPackBlockId::InputSlotsCanonicalRefs => ContextPackBlockRecipe {
                summary_compact_options: None,
                preferred_level: if matches!(
                    mode,
                    ContextPressureMode::Critical | ContextPressureMode::Medium
                ) {
                    ContextCompressLevel::Skeleton
                } else {
                    ContextCompressLevel::Full
                },
            },
        }
    }
}

fn usage_field_u64(usage: Option<&Value>, key: &str) -> Option<u64> {
    usage
        .and_then(|value| value.get(key))
        .and_then(Value::as_u64)
}

fn budget_from_ratio_bps(ratio_bps: u64, min_budget: usize, max_budget: usize) -> usize {
    if ratio_bps <= ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS {
        return min_budget;
    }
    if ratio_bps >= ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_RELAXED_THRESHOLD_BPS {
        return max_budget;
    }
    let span_bps = ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_RELAXED_THRESHOLD_BPS
        - ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS;
    let progress_bps = ratio_bps
        .saturating_sub(ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_TIGHT_THRESHOLD_BPS);
    let span_tokens = max_budget.saturating_sub(min_budget);
    min_budget
        + usize::try_from(
            progress_bps.saturating_mul(u64::try_from(span_tokens).unwrap_or(0)) / span_bps,
        )
        .unwrap_or(0)
}

fn budget_from_remaining_tokens(remaining_tokens: u64) -> usize {
    if remaining_tokens <= ToolMemoryBudgetPolicy::TOOL_MEMORY_REMAINING_ABS_MIN {
        return ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MIN_TOKENS;
    }
    if remaining_tokens >= ToolMemoryBudgetPolicy::TOOL_MEMORY_REMAINING_ABS_MAX {
        return ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MAX_TOKENS;
    }
    let span_remaining = ToolMemoryBudgetPolicy::TOOL_MEMORY_REMAINING_ABS_MAX
        - ToolMemoryBudgetPolicy::TOOL_MEMORY_REMAINING_ABS_MIN;
    let progress =
        remaining_tokens.saturating_sub(ToolMemoryBudgetPolicy::TOOL_MEMORY_REMAINING_ABS_MIN);
    let span_tokens = ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MAX_TOKENS
        - ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MIN_TOKENS;
    ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_MIN_TOKENS
        + usize::try_from(
            progress.saturating_mul(u64::try_from(span_tokens).unwrap_or(0)) / span_remaining,
        )
        .unwrap_or(0)
}

fn budget_bounds_from_soft_limit(soft_limit_tokens: u64) -> (usize, usize) {
    let min_budget = usize::try_from(
        soft_limit_tokens.saturating_mul(
            ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_SOFT_LIMIT_MIN_RATIO_BPS,
        ) / 10_000,
    )
    .unwrap_or(usize::MAX)
    .clamp(
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_ABS_MIN_TOKENS,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_ABS_MAX_TOKENS,
    );
    let max_budget = usize::try_from(
        soft_limit_tokens.saturating_mul(
            ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_SOFT_LIMIT_MAX_RATIO_BPS,
        ) / 10_000,
    )
    .unwrap_or(usize::MAX)
    .clamp(
        min_budget,
        ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_ABS_MAX_TOKENS,
    );
    (min_budget, max_budget)
}

fn usage_inputs(
    planner_usage: Option<&Value>,
    runtime_usage: Option<&Value>,
) -> (Option<u64>, Option<u64>, Option<u64>) {
    let remaining_tokens = usage_field_u64(planner_usage, "context_remaining_tokens")
        .or_else(|| usage_field_u64(runtime_usage, "context_remaining_tokens"));
    let soft_limit_tokens = usage_field_u64(planner_usage, "context_soft_limit_tokens")
        .or_else(|| usage_field_u64(runtime_usage, "context_soft_limit_tokens"));
    let usage_ratio_bps = match (remaining_tokens, soft_limit_tokens) {
        (Some(remaining), Some(soft_limit)) if soft_limit > 0 => {
            Some(10_000_u64.saturating_sub(remaining.saturating_mul(10_000) / soft_limit))
        }
        _ => None,
    };
    (remaining_tokens, soft_limit_tokens, usage_ratio_bps)
}

fn interpolate_usize(min_value: usize, max_value: usize, progress_bps: u64) -> usize {
    if min_value >= max_value {
        return min_value;
    }
    let span = max_value.saturating_sub(min_value);
    let offset = usize::try_from(
        u64::try_from(span)
            .unwrap_or(0)
            .saturating_mul(progress_bps.min(10_000))
            / 10_000,
    )
    .unwrap_or(0);
    min_value.saturating_add(offset)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextPressureMode {
    Normal,
    Light,
    Medium,
    Critical,
}

impl ContextPressureMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Critical => "critical",
        }
    }

    pub(crate) fn from_str(mode: &str) -> Option<Self> {
        match mode {
            "critical" => Some(Self::Critical),
            "medium" => Some(Self::Medium),
            "light" => Some(Self::Light),
            "normal" => Some(Self::Normal),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlanningMemoryStoreBudget, ToolMemoryBudgetPolicy};
    use serde_json::json;

    #[test]
    fn planning_memory_store_budget_defaults_without_usage() {
        let budget = ToolMemoryBudgetPolicy::derive_planning_memory_store_budget(None, None);
        assert_eq!(
            budget,
            PlanningMemoryStoreBudget {
                max_entries: ToolMemoryBudgetPolicy::PLANNING_MEMORY_STORE_DEFAULT_MAX_ENTRIES,
                max_entry_chars:
                    ToolMemoryBudgetPolicy::PLANNING_MEMORY_STORE_DEFAULT_MAX_ENTRY_CHARS,
                max_total_chars:
                    ToolMemoryBudgetPolicy::PLANNING_MEMORY_STORE_DEFAULT_MAX_TOTAL_CHARS,
            }
        );
    }

    #[test]
    fn planning_memory_store_budget_tightens_under_critical_pressure() {
        let runtime_usage = json!({
            "context_soft_limit_tokens": 100_000,
            "context_remaining_tokens": 2_000
        });
        let budget =
            ToolMemoryBudgetPolicy::derive_planning_memory_store_budget(None, Some(&runtime_usage));
        assert!(budget.max_entries <= 16);
        assert!(budget.max_entry_chars <= 3_000);
        assert!(budget.max_total_chars <= 40_000);
    }
}
