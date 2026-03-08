use super::brain::{DecisionPolicy, LlmBrain};
use super::intent_segmented::SegmentedIntentPlanner;
use super::r#loop::{run_agent_loop, AgentLoopConfig, CommandBuilder};
use super::summary::PauseKind;
use crate::checkpoint_ledger::RunnerCheckpointLedger;
use crate::cli::{AgentCommand, AgentProfile, OutputFormat};
use crate::config::{
    ChainConfig, RunnerConfig, RunnerEngineConfig, RunnerLlmConfig, RunnerLlmRotationMode,
    RunnerPluginsConfig, SignerConfig,
};
use crate::error::RunnerError;
use ais_engine::{
    create_checkpoint_document, load_checkpoint_from_path, run_plan_once, save_checkpoint_to_path,
    CheckpointEngineState, DefaultSolver, EngineCommandEnvelope, EngineRunStatus,
    EngineRunnerOptions, EngineRunnerState, Executor, ExecutorOutput, RouterExecutor,
};
use ais_llm::{CompleteWithToolsResponse, ScriptedLlmProvider, ToolCall};
use ais_sdk::PlanDocument;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestExecutor;

impl Executor for TestExecutor {
    fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        Ok(ExecutorOutput {
            result: json!({"ok": true}),
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct SegmentedFixtureExecutor;

impl Executor for SegmentedFixtureExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = if node_id.ends_with("q_native_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"200"}
            })
        } else if node_id.ends_with("q_token_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"260"}
            })
        } else {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"tx_hash":"0xsegmented_transfer"}
            })
        };
        Ok(ExecutorOutput {
            result,
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct SegmentedUntilRetryExecutor;

impl Executor for SegmentedUntilRetryExecutor {
    fn execute(&self, node: &Value, runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let node_id = node
            .as_object()
            .and_then(|object| object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = if node_id.ends_with("q_native_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"200"}
            })
        } else if node_id.ends_with("q_token_balance") {
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{"balance":"260"}
            })
        } else {
            let escaped = node_id.replace('~', "~0").replace('/', "~1");
            let seen_before = runtime
                .pointer(format!("/nodes/{escaped}/outputs/outputs/confirmed").as_str())
                .is_some();
            json!({
                "execution_type":"offchain_apy_query",
                "outputs":{
                    "tx_hash":"0xsegmented_transfer_retry",
                    "confirmed": seen_before
                }
            })
        };
        Ok(ExecutorOutput {
            result,
            writes: Map::new(),
            side_effects: Vec::new(),
        })
    }
}

struct ApproveOnceBrain {
    approved: bool,
}

impl DecisionPolicy for ApproveOnceBrain {
    fn decide(
        &mut self,
        summary: &super::summary::PauseSummary,
        commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
        assert_eq!(summary.kind, PauseKind::NeedUserConfirm);
        let node_id = summary.node_id.as_deref().expect("node_id must exist");
        assert!(
            !self.approved,
            "brain must only be invoked once for this plan"
        );
        self.approved = true;
        Ok(vec![commands.user_confirm(node_id, "approve")])
    }
}

struct PanicBrain;

impl DecisionPolicy for PanicBrain {
    fn decide(
        &mut self,
        _summary: &super::summary::PauseSummary,
        _commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
        panic!("brain must not be invoked for this test");
    }
}

include!("tests/mod.rs");

fn write_temp_file(prefix: &str, content: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    path.push(format!(
        "ais-runner-agent-{prefix}-{}-{nanos}.tmp",
        std::process::id()
    ));
    fs::write(&path, content).expect("write temp file");
    path
}
