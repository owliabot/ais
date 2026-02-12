/**
 * @owliabot/ais-ts-sdk
 * TypeScript SDK for parsing and validating AIS (Agent Interaction Specification) files
 */

// Schema types and validators
export {
  // Common
  ChainIdSchema,
  HexAddressSchema,
  AddressSchema,
  AssetSchema,
  TokenAmountSchema,
  AISTypeSchema,
  DetectSchema,
  ValueRefSchema,
  type ChainId,
  type Asset,
  type TokenAmount,
  type AISType,
  type Detect,
  type ValueRef,

  // Execution
  ExecutionSpecSchema,
  ExecutionBlockSchema,
  type JsonAbiParam,
  type JsonAbiFunction,
  type ExecutionSpec,
  type ExecutionBlock,
  type CompositeStep,
  type EvmRead,
  type EvmMultiread,
  type EvmCall,
  type EvmMulticall,
  type Composite,
  type SolanaInstruction,
  type SolanaRead,
  type BitcoinPsbt,
  CORE_EXECUTION_TYPES,
  type CoreExecutionType,
  type PluginExecutionSpec,

  // Protocol
  ProtocolSpecSchema,
  type ProtocolSpec,
  type Meta,
  type Deployment,
  type Param,
  type ParamConstraints,
  type Query,
  type Action,
  type ProtocolRisk,
  type HardConstraints,
  type CalculatedField,
  type CalculatedFields,
  type AssetMapping,
  type TestVector,
  type ReturnField,
  type Consistency,
  type RiskTag,

  // Pack
  PackSchema,
  type Pack,
  type PackMeta,
  type ProtocolInclude,
  type Policy,
  type HardConstraintsDefaults,
  type TokenPolicy,
  type TokenAllowlistEntry,
  type Providers,
  type ActionOverride,

  // Workflow
  WorkflowSchema,
  type Workflow,
  type WorkflowMeta,
  type WorkflowInput,
  type WorkflowNode,
  type WorkflowPolicy,
  type PackRef,
  type CalculatedOverride,

  // Union
  AISDocumentSchema,
  type AnyAISDocument,
  type AISSchemaType,
} from './schema/index.js';

// Catalog (cards for search)
export {
  CatalogSchemaVersion,
  CatalogSchema,
  ActionCardSchema,
  QueryCardSchema,
  PackCardSchema,
  CatalogIndexSchemaVersion,
  buildCatalog,
  buildCatalogIndex,
  filterByPack,
  filterByEngineCapabilities,
  type Catalog,
  type CatalogDocumentEntry,
  type ActionCard,
  type QueryCard,
  type PackCard,
  type CatalogIndex,
  type EngineCapabilities,
  type DetectProviderCandidate,
  type ExecutionPluginCandidate,
  ExecutableCandidatesSchemaVersion,
  getExecutableCandidates,
  type ExecutableCandidates,
  type ExecutableActionCandidate,
  type ExecutableQueryCandidate,
  type ExecutableDetectProviderCandidate,
  type ExecutableExecutionPluginCandidate,
} from './catalog/index.js';

// Deterministic agent loop reference (AGT107)
export {
  runDeterministicAgentLoop,
  type DeterministicAgentConfig,
  type DeterministicAgentResult,
  type DeterministicAgentCommand,
} from './agent/index.js';

// Plan skeleton (agent-facing minimal plan contract)
export {
  PlanSkeletonSchemaVersion,
  PlanSkeletonSchema,
  PlanSkeletonNodeSchema,
  compilePlanSkeleton,
  type PlanSkeleton,
  type PlanSkeletonNode,
  type PlanSkeletonCompileIssue,
  type CompilePlanSkeletonResult,
} from './skeleton/index.js';

// Plugins
export {
  createExecutionTypeRegistry,
  defaultExecutionTypeRegistry,
  registerExecutionType,
  validateProtocolExecutionTypes,
  UnknownExecutionTypeError,
  PluginExecutionSchemaError,
  type ExecutionTypePlugin,
  type ExecutionTypeRegistry,
} from './plugins/index.js';

// Parser
export {
  parseAIS,
  parseProtocolSpec,
  parsePack,
  parseWorkflow,
  detectType,
  validate,
  AISParseError,
  type ParseOptions,
} from './parser.js';

// Resolver
export {
  // Context
  createContext,
  getRuntimeRoot,
  getRef,
  setRef,
  setQueryResult,
  setNodeOutputs,
  type ResolverContext,
  type RuntimeContext,
  type RuntimeNodeState,

  // References
  registerProtocol,
  parseProtocolRef,
  resolveProtocolRef,
  resolveAction,
  resolveQuery,
  expandPack,
  getContractAddress,
  getSupportedChains,

  // Expressions
  hasExpressions,
  extractExpressions,
  resolveExpression,
  resolveExpressionString,
  resolveExpressionObject,

  // ValueRef evaluation
  evaluateValueRef,
  evaluateValueRefAsync,
  ValueRefEvalError,
  type DetectResolver,
  type EvaluateValueRefOptions,
} from './resolver/index.js';

// Validator
export {
  // Constraints
  validateConstraints,
  getHardConstraints,
  type ConstraintInput,
  type ConstraintViolation,
  type ConstraintResult,

  // Workflow
  validateWorkflow,
  getWorkflowDependencies,
  getWorkflowProtocols,
  getExecutionOrder,
  type WorkflowIssue,
  type WorkflowValidationResult,

  // Workspace (cross-file)
  validateWorkspaceReferences,
  type WorkspaceIssue,
  type WorkspaceIssueSeverity,
  type WorkspaceDocuments,

  // Lint
  lintDocument,
  type LintIssue,
  type LintRule,
  type LintSeverity,

  // Plugins
  createValidatorRegistry,
  defaultValidatorRegistry,
  registerValidatorPlugin,
  type ValidatorPlugin,
  type ValidatorRegistry,
} from './validator/index.js';

// Structured issues (AGT104)
export {
  StructuredIssueSchema,
  StructuredIssueSeveritySchema,
  StructuredIssueRelatedSchema,
  zodPathToFieldPath,
  issueLocator,
  fromWorkspaceIssues,
  fromWorkflowIssues,
  fromZodError,
  fromPlanBuildError,
  type StructuredIssue,
  type StructuredIssueSeverity,
  type StructuredIssueRelated,
} from './issues/index.js';

// Policy enforcement (agent/runner integration)
export {
  checkDetectAllowed,
  pickDetectProvider,
  checkExecutionPluginAllowed,
  compileWritePreview,
  extractPolicyGateInput,
  enforcePolicyGate,
  explainPolicyGateResult,
  type EnforcementKind,
  type EnforcementResult,
  type DetectAllowInput,
  type DetectProviderPickInput,
  type DetectProviderPickResult,
  type ExecutionPluginAllowInput,
  type WritePreview,
  type CompileWritePreviewOptions,
  type PolicyGateInput,
  type ExtractPolicyGateInputOptions,
  type EnforcePolicyGateOptions,
  PolicyGateInputSchema,
  PolicyGateOutputSchema,
  POLICY_GATE_INPUT_FIELD_DICTIONARY,
  POLICY_GATE_OUTPUT_FIELD_DICTIONARY,
  validatePolicyGateInput,
  validatePolicyGateOutput,
  type PolicyGateInputShape,
  type PolicyGateOutputShape,
  type PolicyFieldNullSemantics,
  type PolicyGateFieldDictionaryEntry,
} from './policy/index.js';

// Loader
export {
  loadFile,
  loadProtocol,
  loadPack,
  loadWorkflow,
  loadWorkflowBundle,
  loadDirectory,
  loadDirectoryAsContext,
  type LoadResult,
  type LoadError,
  type DirectoryLoadResult,
  type LoadWorkflowBundleOptions,
  type WorkflowBundleLoadResult,
} from './loader.js';

// Execution
export {
  buildTransaction,
  buildQuery,
  buildWorkflowTransactions,
  solana,
  evm,
  encodeFunctionSelector,
  encodeJsonAbiFunctionCall,
  decodeJsonAbiFunctionResult,
  buildFunctionSignatureFromJsonAbi,
  AbiArgsError,
  AbiEncodingError,
  AbiDecodingError,
  compileEvmExecution,
  compileEvmExecutionAsync,
  compileEvmCall,
  compileEvmCallAsync,
  compileEvmRead,
  compileEvmReadAsync,
  type CompileEvmOptions,
  type CompiledEvmRequest,
  EvmCompileError,
  keccak256,
  ExecutionPlanSchema,
  ExecutionPlanNodeSchema,
  buildWorkflowExecutionPlan,
  selectExecutionSpec,
  getNodeReadiness,
  getNodeReadinessAsync,
  type ExecutionPlan,
  type ExecutionPlanNode,
  type PlanWrite,
  type NodeReadinessResult,
  type NodeRunState,
  PlanBuildError,
  type TransactionRequest,
  type BuildOptions,
  type BuildResult,
  type BuildError,
  type BuildOutput,
} from './execution/index.js';

// Registry
export {
  canonicalizeJcs,
  specHashKeccak256,
  JcsCanonicalizeError,
} from './registry/index.js';

// Detect
export {
  DetectProviderRegistry,
  createDetectProviderRegistry,
  defaultDetectProviderRegistry,
  registerDetectProvider,
  createDetectResolver,
  type DetectProvider,
  type CreateDetectResolverOptions,
} from './detect/index.js';

// Engine (interfaces + patches)
export {
  type RuntimePatchOp,
  type RuntimePatch,
  RuntimePatchSchema,
  RuntimePatchOpSchema,
  validateRuntimePatch,
  checkRuntimePatchPathAllowed,
  DEFAULT_RUNTIME_PATCH_GUARD_POLICY,
  buildRuntimePatchGuardPolicy,
  type RuntimePatchValidationError,
  type RuntimePatchGuardPolicy,
  type RuntimePatchGuardOptions,
  type RuntimePatchGuardRejection,
  type RuntimePatchAuditEntry,
  type RuntimePatchAuditSummary,
  RuntimePatchError,
  applyRuntimePatch,
  applyRuntimePatches,
  type ApplyRuntimePatchOptions,
  type ApplyRuntimePatchResult,
  type EngineEvent,
  type EngineCheckpoint,
  type CheckpointStore,
  type SolverResult,
  type Solver,
  type ExecutorResult,
  type Executor,
  EvmJsonRpcExecutor,
  SolanaRpcExecutor,
  type SolanaRpcExecutorOptions,
  type SolanaRpcConnectionLike,
  type SolanaSigner,
  type JsonRpcTransport,
  type EvmSigner,
  type EvmTxRequest,
  type EvmJsonRpcExecutorOptions,
  solver,
  createSolver,
  type SolverOptions,
  runPlan,
  type RunPlanOptions,
  serializeCheckpoint,
  deserializeCheckpoint,
  checkpointJsonReplacer,
  checkpointJsonReviver,
  type SerializeCheckpointOptions,
  AIS_JSON_TYPE_KEY,
  AIS_JSON_CODEC_PROFILE_VERSION,
  AIS_JSON_CODEC_PROFILE,
  aisJsonCodec,
  createAisJsonReplacer,
  type AisJsonCodecProfile,
  type AisJsonCodec,
  stringifyAisJson,
  parseAisJson,
  aisJsonReplacer,
  aisJsonReviver,
  type ExecutionTraceSink,
  type ExecutionTraceRecord,
  type TraceRedactMode,
  createJsonlTraceSink,
  createJsonlTraceSinkFromWritable,
  redactEngineEventForTrace,
  redactEngineEventByMode,
  createEngineEventJsonlWriter,
  engineEventToEnvelope,
  engineEventsToJsonl,
  encodeEngineEventJsonlRecord,
  decodeEngineEventJsonlRecord,
  type EngineEventJsonlRecord,
  type EngineEventEnvelope,
  RunnerCommandSchema,
  RunnerCommandKindSchema,
  validateRunnerCommand,
  summarizeCommand,
  commandPatches,
  type RunnerCommandKind,
  type RunnerCommand,
  type ParsedRunnerCommand,
  type CommandValidationError,
  createJsonlRpcPeer,
  type JsonlRpcPeer,
  type JsonlRpcPeerOptions,

  // Confirmation summary (AGT106)
  ConfirmationSummarySchemaVersion,
  ConfirmationSummarySchema,
  summarizeNeedUserConfirm,
  type ConfirmationSummary,
} from './engine/index.js';

// CEL Expression Evaluator
export {
  Evaluator as CELEvaluator,
  evaluateCEL,
  Lexer as CELLexer,
  Parser as CELParser,
  type CELValue,
  type CELContext,
  type Token as CELToken,
  type ASTNode as CELASTNode,
} from './cel/index.js';

// Builder DSL
export {
  protocol,
  pack,
  workflow,
  param,
  output,
  ProtocolBuilder,
  PackBuilder,
  WorkflowBuilder,
  type ParamDef,
  type OutputDef,
} from './builder/index.js';
