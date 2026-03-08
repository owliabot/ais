use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ContextBlockPriority {
    Medium,
    Low,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PackPhaseHint {
    #[default]
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextPackBlockId {
    PreviousError,
    PreviousErrorAutofillHistory,
    RecoveryDiagnostics,
    ToolMemoryProjection,
    InputStoreFacts,
    CapabilityViewProtocols,
    PreviousErrorLastFailedFinalize,
}

impl ContextPackBlockId {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PreviousError => "previous_error",
            Self::PreviousErrorAutofillHistory => "previous_error.autofill_history",
            Self::RecoveryDiagnostics => "recovery_diagnostics",
            Self::ToolMemoryProjection => "tool_memory_projection",
            Self::InputStoreFacts => "input_store.facts",
            Self::CapabilityViewProtocols => "capability_view.protocols",
            Self::PreviousErrorLastFailedFinalize => "previous_error.last_failed_finalize",
        }
    }

    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::PreviousError => "/previous_error",
            Self::PreviousErrorAutofillHistory => "/previous_error/autofill_history",
            Self::RecoveryDiagnostics => "/recovery_diagnostics",
            Self::ToolMemoryProjection => "/tool_memory_projection",
            Self::InputStoreFacts => "/input_store/facts",
            Self::CapabilityViewProtocols => "/capability_view/protocols",
            Self::PreviousErrorLastFailedFinalize => "/previous_error/last_failed_finalize",
        }
    }

    pub(crate) const fn default_priority(self) -> ContextBlockPriority {
        match self {
            Self::PreviousError => ContextBlockPriority::Stale,
            Self::PreviousErrorAutofillHistory => ContextBlockPriority::Stale,
            Self::RecoveryDiagnostics => ContextBlockPriority::Low,
            Self::ToolMemoryProjection => ContextBlockPriority::Low,
            Self::InputStoreFacts => ContextBlockPriority::Medium,
            Self::CapabilityViewProtocols => ContextBlockPriority::Low,
            Self::PreviousErrorLastFailedFinalize => ContextBlockPriority::Stale,
        }
    }

    pub(crate) fn priority_for_phase(self, phase: PackPhaseHint) -> ContextBlockPriority {
        let _ = phase;
        self.default_priority()
    }

    pub(crate) const fn is_evictable(self) -> bool {
        true
    }

    pub(crate) const fn optional_pack_blocks() -> &'static [ContextPackBlockId] {
        &[
            ContextPackBlockId::PreviousError,
            ContextPackBlockId::PreviousErrorAutofillHistory,
            ContextPackBlockId::RecoveryDiagnostics,
            ContextPackBlockId::ToolMemoryProjection,
            ContextPackBlockId::InputStoreFacts,
            ContextPackBlockId::CapabilityViewProtocols,
            ContextPackBlockId::PreviousErrorLastFailedFinalize,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextCompressLevel {
    Full,
    Summary,
    Skeleton,
}

impl ContextCompressLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Summary => "summary",
            Self::Skeleton => "skeleton",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackAction {
    Keep,
    Compress,
    Drop,
}

impl PackAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Compress => "compress",
            Self::Drop => "drop",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PackDecision {
    pub(crate) block_id: String,
    pub(crate) action: PackAction,
    pub(crate) reason: &'static str,
    pub(crate) before_level: Option<ContextCompressLevel>,
    pub(crate) after_level: Option<ContextCompressLevel>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PackDiagnostics {
    pub(crate) packed_blocks_total: u64,
    pub(crate) packed_blocks_included: u64,
    pub(crate) packed_blocks_evicted: u64,
    pub(crate) compressed_blocks_total: u64,
    pub(crate) compressed_by_reason: BTreeMap<String, u64>,
    pub(crate) evicted_by_reason: BTreeMap<String, u64>,
}

impl PackDiagnostics {
    pub(crate) fn observe_decision(&mut self, decision: &PackDecision) {
        match decision.action {
            PackAction::Compress => {
                self.compressed_blocks_total = self.compressed_blocks_total.saturating_add(1);
                let entry = self
                    .compressed_by_reason
                    .entry(decision.reason.to_string())
                    .or_insert(0);
                *entry = entry.saturating_add(1);
            }
            PackAction::Drop => {
                let entry = self
                    .evicted_by_reason
                    .entry(decision.reason.to_string())
                    .or_insert(0);
                *entry = entry.saturating_add(1);
            }
            PackAction::Keep => {}
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PackTrace {
    pub(crate) decisions: Vec<PackDecision>,
}

impl PackTrace {
    pub(crate) fn push(&mut self, decision: PackDecision) {
        self.decisions.push(decision);
    }
}
