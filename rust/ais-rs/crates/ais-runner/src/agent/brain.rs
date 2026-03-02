use crate::cli::ApprovalsMode;
use crate::error::RunnerError;
use ais_engine::{decode_command_jsonl_line, EngineCommandEnvelope};
use ais_llm::{CompleteWithToolsRequest, LlmMessage, LlmProvider, MessageRole, ToolCall, ToolSpec};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, Write};

use super::budget::compact_json_for_llm;
use super::candidates::CandidateContext;
use super::r#loop::CommandBuilder;
use super::sanitize::sanitize_for_llm_payload;
use super::summary::{PauseKind, PauseSummary};

const DEFAULT_AGENT_CONTROLLER_SYSTEM_PROMPT: &str =
    "You are an AIS agent controller. Respond only with tool calls that map to engine commands.";

pub trait DecisionPolicy {
    fn decide(
        &mut self,
        summary: &PauseSummary,
        commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionPath {
    YoloAutoApprove,
    AssistLlmAutoApprove,
    ManualPrompt,
}

pub struct AgentDecisionPolicy<P> {
    approvals_mode: ApprovalsMode,
    assist_threshold: Option<u8>,
    llm: Option<LlmBrain<P>>,
    manual_always_approve_this_run: bool,
}

impl<P> AgentDecisionPolicy<P> {
    pub fn new(
        approvals_mode: ApprovalsMode,
        assist_threshold: Option<u8>,
        llm: Option<LlmBrain<P>>,
    ) -> Self {
        Self {
            approvals_mode,
            assist_threshold,
            llm,
            manual_always_approve_this_run: false,
        }
    }

    pub fn classify_path(&self, summary: &PauseSummary) -> DecisionPath {
        if self.approvals_mode == ApprovalsMode::Yolo
            && summary.kind == PauseKind::NeedUserConfirm
            && summary.node_id.is_some()
        {
            return DecisionPath::YoloAutoApprove;
        }

        if self.approvals_mode == ApprovalsMode::Assist
            && self.assist_threshold.is_some()
            && self.llm.is_some()
            && should_attempt_assist_auto_approve(
                summary,
                self.assist_threshold.unwrap_or_default(),
            )
        {
            return DecisionPath::AssistLlmAutoApprove;
        }

        DecisionPath::ManualPrompt
    }
}

impl<P> DecisionPolicy for AgentDecisionPolicy<P>
where
    P: LlmProvider,
{
    fn decide(
        &mut self,
        summary: &PauseSummary,
        commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
        if self.manual_always_approve_this_run
            && summary.kind == PauseKind::NeedUserConfirm
            && summary.node_id.is_some()
        {
            let node_id = summary.node_id.as_deref().unwrap_or_default();
            eprintln!("[agent] manual always_approve_this_run node={node_id}");
            return Ok(vec![commands.user_confirm(node_id, "approve")]);
        }
        match self.classify_path(summary) {
            DecisionPath::YoloAutoApprove => {
                let node_id = summary.node_id.as_deref().unwrap_or_default();
                eprintln!("[agent] yolo auto-approve need_user_confirm node={node_id}");
                Ok(vec![commands.user_confirm(node_id, "approve")])
            }
            DecisionPath::AssistLlmAutoApprove => {
                let threshold = self.assist_threshold.unwrap_or_default();
                if let Some(llm) = self.llm.as_mut() {
                    match llm.decide_with_tools(summary, commands) {
                        Ok(out) => {
                            eprintln!(
                                "[agent] assist llm auto-approve applied (risk<=threshold {})",
                                threshold
                            );
                            Ok(out)
                        }
                        Err(error) => {
                            eprintln!(
                                "[agent] assist llm auto-approve failed: {error}; fallback to manual confirm"
                            );
                            prompt_human_decision(
                                summary,
                                commands,
                                &mut self.manual_always_approve_this_run,
                            )
                        }
                    }
                } else {
                    prompt_human_decision(
                        summary,
                        commands,
                        &mut self.manual_always_approve_this_run,
                    )
                }
            }
            DecisionPath::ManualPrompt => {
                prompt_human_decision(summary, commands, &mut self.manual_always_approve_this_run)
            }
        }
    }
}

fn prompt_human_decision(
    summary: &PauseSummary,
    commands: &mut CommandBuilder,
    manual_always_approve_this_run: &mut bool,
) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
    eprintln!("{}", summary.render_for_humans());
    loop {
        eprint!("[agent] next action (help/approve/deny/cancel/jsonl): ");
        io::stderr()
            .flush()
            .map_err(|error| RunnerError::EventsIo(error.to_string()))?;

        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|error| RunnerError::EventsIo(error.to_string()))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match line {
            "help" | "h" => {
                eprintln!("help:");
                eprintln!("- approve|a     (need_user_confirm) approve current node");
                eprintln!("- deny|d        (need_user_confirm) deny current node");
                eprintln!(
                    "- always_approve_this_run|aa  auto approve remaining confirmations this run"
                );
                eprintln!("- cancel|c      send cancel command");
                eprintln!("- jsonl <line>  paste one engine command JSON line");
                eprintln!("- <json>        paste raw engine command JSON line");
                continue;
            }
            "cancel" | "c" | "quit" | "q" => return Ok(vec![commands.cancel()]),
            "approve" | "a" => {
                let Some(node_id) = summary.node_id.as_deref() else {
                    eprintln!("[agent] no node_id available for approve");
                    continue;
                };
                return Ok(vec![commands.user_confirm(node_id, "approve")]);
            }
            "deny" | "d" => {
                let Some(node_id) = summary.node_id.as_deref() else {
                    eprintln!("[agent] no node_id available for deny");
                    continue;
                };
                return Ok(vec![commands.user_confirm(node_id, "deny")]);
            }
            "always_approve_this_run" | "always" | "aa" => {
                let Some(node_id) = summary.node_id.as_deref() else {
                    eprintln!("[agent] no node_id available for always_approve_this_run");
                    continue;
                };
                *manual_always_approve_this_run = true;
                eprintln!("[agent] enabled always_approve_this_run for this process");
                return Ok(vec![commands.user_confirm(node_id, "approve")]);
            }
            _ => {}
        }

        if let Some(rest) = line.strip_prefix("jsonl ") {
            return Ok(vec![decode_jsonl(rest)?]);
        }
        if line.starts_with('{') {
            return Ok(vec![decode_jsonl(line)?]);
        }

        eprintln!("[agent] unknown input: `{line}` (type `help`)");
    }
}

fn decode_jsonl(line: &str) -> Result<EngineCommandEnvelope, RunnerError> {
    decode_command_jsonl_line(line)
        .map_err(|error| RunnerError::EventsIo(format!("command decode failed: {error}")))
}

pub struct LlmBrain<P> {
    provider: P,
    system_prompt: String,
    candidate_context: Option<CandidateContext>,
    max_tool_rounds: u8,
}

impl<P> LlmBrain<P> {
    pub fn default_system_prompt() -> &'static str {
        DEFAULT_AGENT_CONTROLLER_SYSTEM_PROMPT
    }

    pub fn new(provider: P) -> Self {
        Self {
            provider,
            system_prompt: DEFAULT_AGENT_CONTROLLER_SYSTEM_PROMPT.to_string(),
            candidate_context: None,
            max_tool_rounds: 4,
        }
    }

    pub fn with_system_prompt(mut self, system_prompt: String) -> Self {
        if !system_prompt.trim().is_empty() {
            self.system_prompt = system_prompt;
        }
        self
    }

    pub fn with_candidate_context(mut self, candidate_context: CandidateContext) -> Self {
        self.candidate_context = Some(candidate_context);
        self
    }

    pub fn decide_with_tools(
        &mut self,
        summary: &PauseSummary,
        commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError>
    where
        P: LlmProvider,
    {
        let mut messages = vec![
            LlmMessage {
                role: MessageRole::System,
                content: Some(self.system_prompt.clone()),
                tool_name: None,
                tool_call_id: None,
                tool_calls: vec![],
            },
            LlmMessage {
                role: MessageRole::User,
                content: Some(render_pause_for_llm(
                    summary,
                    self.candidate_context
                        .as_ref()
                        .map(|context| &context.index_candidates),
                )),
                tool_name: None,
                tool_call_id: None,
                tool_calls: vec![],
            },
        ];
        let tools = llm_tools(self.candidate_context.is_some());

        for _ in 0..self.max_tool_rounds {
            let response = self
                .provider
                .complete_with_tools(CompleteWithToolsRequest {
                    messages: messages.clone(),
                    tools: tools.clone(),
                })
                .map_err(|error| RunnerError::Llm(error.to_string()))?;
            if response.tool_calls.is_empty() {
                return Err(RunnerError::Llm(
                    "provider returned no tool calls while paused".to_string(),
                ));
            }

            messages.push(LlmMessage {
                role: MessageRole::Assistant,
                content: response.assistant_content.clone(),
                tool_name: None,
                tool_call_id: None,
                tool_calls: response.tool_calls.clone(),
            });

            let mut engine_commands = Vec::<EngineCommandEnvelope>::new();
            let mut tool_results = Vec::<LlmMessage>::new();
            for tool_call in &response.tool_calls {
                match decode_tool_call(
                    tool_call,
                    summary,
                    commands,
                    self.candidate_context.as_ref(),
                )? {
                    DecodedToolCall::Engine(command) => engine_commands.push(command),
                    DecodedToolCall::ToolResult {
                        tool_name,
                        tool_call_id,
                        content,
                    } => {
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
            if !engine_commands.is_empty() {
                return Ok(engine_commands);
            }
            if tool_results.is_empty() {
                return Err(RunnerError::Llm(
                    "provider returned no actionable tool calls".to_string(),
                ));
            }
            messages.extend(tool_results);
        }

        Err(RunnerError::Llm(
            "llm tool round limit reached without engine commands".to_string(),
        ))
    }
}

impl<P> DecisionPolicy for LlmBrain<P>
where
    P: LlmProvider,
{
    fn decide(
        &mut self,
        summary: &PauseSummary,
        commands: &mut CommandBuilder,
    ) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
        self.decide_with_tools(summary, commands)
    }
}

fn llm_tools(enable_candidate_detail_tool: bool) -> Vec<ToolSpec> {
    let mut tools = vec![
        ToolSpec {
            name: "confirm".to_string(),
            description: "Approve or deny current need_user_confirm node".to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{
                "decision":{"type":"string","enum":["approve","deny"]},
                "node_id":{"type":"string"}
              },
              "required":["decision"],
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "cancel".to_string(),
            description: "Cancel current run".to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{},
              "additionalProperties":false
            }),
        },
        ToolSpec {
            name: "send_engine_command".to_string(),
            description: "Send a full engine command envelope when built-in tools are insufficient"
                .to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{
                "command":{"type":"object"}
              },
              "required":["command"],
              "additionalProperties":false
            }),
        },
    ];
    if enable_candidate_detail_tool {
        tools.push(ToolSpec {
            name: "get_candidate_detail".to_string(),
            description: "Fetch detail cards by candidate refs".to_string(),
            input_schema: json!({
              "type":"object",
              "properties":{
                "refs":{"type":"array","items":{"type":"string"}}
              },
              "required":["refs"],
              "additionalProperties":false
            }),
        });
    }
    tools
}

fn render_pause_for_llm(summary: &PauseSummary, index_candidates: Option<&Value>) -> String {
    let compact_candidates = index_candidates.map(|value| {
        let sanitized = sanitize_for_llm_payload(value);
        compact_json_for_llm(&sanitized)
    });
    serde_json::to_string_pretty(&json!({
        "paused_reason": summary.raw_reason,
        "kind": format!("{:?}", summary.kind),
        "node_id": summary.node_id,
        "need_user_confirm": summary.need_user_confirm,
        "last_error_reason": summary.last_error_reason,
        "index_candidates": compact_candidates,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn should_attempt_assist_auto_approve(summary: &PauseSummary, threshold: u8) -> bool {
    if summary.kind != PauseKind::NeedUserConfirm {
        return false;
    }
    let risk = summary
        .need_user_confirm
        .as_ref()
        .and_then(|need| need.confirmation_summary.as_ref())
        .and_then(|value| value.as_object())
        .and_then(|obj| obj.get("risk_level"))
        .and_then(|value| match value {
            serde_json::Value::Number(number) => {
                number.as_u64().and_then(|risk| u8::try_from(risk).ok())
            }
            _ => None,
        });
    risk.is_some_and(|risk| risk <= threshold)
}

#[derive(Debug, Deserialize)]
struct ConfirmToolArgs {
    decision: String,
    #[serde(default)]
    node_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendEngineCommandArgs {
    command: EngineCommandEnvelope,
}

#[derive(Debug, Deserialize)]
struct CandidateDetailToolArgs {
    refs: Vec<String>,
}

enum DecodedToolCall {
    Engine(EngineCommandEnvelope),
    ToolResult {
        tool_name: String,
        tool_call_id: String,
        content: String,
    },
}

fn decode_tool_call(
    tool: &ToolCall,
    summary: &PauseSummary,
    commands: &mut CommandBuilder,
    candidate_context: Option<&CandidateContext>,
) -> Result<DecodedToolCall, RunnerError> {
    match tool.name.as_str() {
        "confirm" => {
            let args: ConfirmToolArgs = serde_json::from_value(tool.arguments.clone())
                .map_err(|error| RunnerError::Llm(format!("invalid confirm args: {error}")))?;
            let decision = match args.decision.as_str() {
                "approve" | "deny" => args.decision.as_str(),
                _ => {
                    return Err(RunnerError::Llm(format!(
                        "invalid confirm decision `{}`",
                        args.decision
                    )))
                }
            };
            let node_id = args
                .node_id
                .as_deref()
                .or(summary.node_id.as_deref())
                .ok_or_else(|| RunnerError::Llm("confirm requires node_id".to_string()))?;
            Ok(DecodedToolCall::Engine(
                commands.user_confirm(node_id, decision),
            ))
        }
        "cancel" => Ok(DecodedToolCall::Engine(commands.cancel())),
        "send_engine_command" => {
            let args: SendEngineCommandArgs = serde_json::from_value(tool.arguments.clone())
                .map_err(|error| {
                    RunnerError::Llm(format!("invalid send_engine_command args: {error}"))
                })?;
            Ok(DecodedToolCall::Engine(args.command))
        }
        "get_candidate_detail" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "candidate detail tool is unavailable".to_string(),
                ));
            };
            let args: CandidateDetailToolArgs = serde_json::from_value(tool.arguments.clone())
                .map_err(|error| {
                    RunnerError::Llm(format!("invalid get_candidate_detail args: {error}"))
                })?;
            let details = context.get_details_for_refs(&args.refs);
            let sanitized = sanitize_for_llm_payload(&details);
            let compacted = compact_json_for_llm(&sanitized);
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            Ok(DecodedToolCall::ToolResult {
                tool_name: "get_candidate_detail".to_string(),
                tool_call_id: tool.id.clone(),
                content,
            })
        }
        other => Err(RunnerError::Llm(format!("unsupported tool `{other}`"))),
    }
}

#[cfg(test)]
#[path = "tests/brain_module.rs"]
mod tests;
