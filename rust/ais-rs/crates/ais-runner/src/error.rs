use ais_core::StructuredIssue;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("read file failed `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("plan parse failed: {0}")]
    PlanParse(String),
    #[error("before plan parse failed: {0}")]
    PlanDiffBeforeParse(String),
    #[error("after plan parse failed: {0}")]
    PlanDiffAfterParse(String),
    #[error("plan file must be AIS plan document")]
    NotPlanDocument,
    #[error("runtime parse failed: {0}")]
    RuntimeParse(String),
    #[error("runner config path is required for plan execution: pass `--config <file>`")]
    MissingRunnerConfig,
    #[error("runner config load failed: {0}")]
    ConfigLoad(String),
    #[error("runner config invalid for plan: {0:?}")]
    ConfigInvalidForPlan(Vec<StructuredIssue>),
    #[error("replay requires `--trace-jsonl <file>` or `--checkpoint <file>`")]
    ReplayInputRequired,
    #[error("replay from checkpoint requires `--plan <file>`")]
    ReplayMissingPlan,
    #[error("replay from checkpoint requires `--config <file>`")]
    ReplayMissingConfig,
    #[error("replay plan parse failed: {0}")]
    ReplayPlanParse(String),
    #[error("replay trace read failed `{path}`: {source}")]
    ReplayTraceRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("replay trace decode failed: {0}")]
    ReplayTraceDecode(String),
    #[error("checkpoint load failed `{path}`: {reason}")]
    CheckpointLoad { path: String, reason: String },
    #[error("checkpoint save failed `{path}`: {reason}")]
    CheckpointSave { path: String, reason: String },
    #[error("write file failed `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("write events JSONL failed: {0}")]
    EventsIo(String),
    #[error("write trace JSONL failed: {0}")]
    TraceIo(String),
    #[error("write agent trace JSONL failed: {0}")]
    AgentTraceIo(String),
    #[error("llm provider failed: {0}")]
    Llm(String),
    #[error("agent profile invalid: {0}")]
    AgentProfile(String),
    #[error("commands stdin jsonl decode failed at line {line}: {reason}")]
    CommandDecode { line: usize, reason: String },
    #[error("replace_plan invalid for command `{command_id}`: {reason}")]
    ReplacePlanInvalid { command_id: String, reason: String },
    #[error("engine run reached iteration limit ({0})")]
    IterationLimitExceeded(usize),
    #[error("json encode failed: {0}")]
    JsonEncode(#[from] serde_json::Error),
    #[error("workflow parse failed: {0}")]
    WorkflowParse(String),
    #[error("workspace load failed: {0}")]
    WorkspaceLoad(String),
    #[error("workspace validation failed: {0}")]
    WorkspaceValidate(String),
    #[error("workflow validation failed: {0}")]
    WorkflowValidate(String),
    #[error("workflow compile failed: {0}")]
    WorkflowCompile(String),
    #[error("workflow outputs evaluation failed: {0}")]
    WorkflowOutputs(String),
    #[error("{command} is not implemented yet")]
    NotImplemented { command: &'static str },
}
