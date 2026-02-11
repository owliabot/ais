import type {
  DetectResolver,
  DirectoryLoadResult,
  EngineCheckpoint,
  EngineEvent,
  Executor,
  ExecutorResult,
  ExecutionPlan,
  ExecutionPlanNode,
  NodeReadinessResult,
  RunPlanOptions,
  ResolverContext,
  RuntimePatch,
  Solver,
  SolverResult,
  WorkspaceDocuments,
  WorkspaceIssue,
  WorkflowIssue,
  WorkflowValidationResult,
  Workflow,
  ValueRef,
  SolanaInstruction,
  Pack,
} from '../../../ts-sdk/dist/index.js';

export type RunnerPlan = ExecutionPlan;
export type RunnerPlanNode = ExecutionPlanNode;
export type RunnerReadiness = NodeReadinessResult;
export type RunnerContext = ResolverContext;
export type RunnerPatch = RuntimePatch;
export type RunnerWorkflow = Workflow;
export type RunnerWorkflowNode = Workflow['nodes'][number];
export type RunnerValueRef = ValueRef;
export type RunnerSolanaInstruction = SolanaInstruction;
export type RunnerPack = Pack;
export type RunnerWorkspaceDocuments = DirectoryLoadResult;
export type RunnerWorkspaceValidationDocuments = WorkspaceDocuments;
export type RunnerWorkspaceIssue = WorkspaceIssue;
export type RunnerWorkflowIssue = WorkflowIssue;
export type RunnerWorkflowValidationResult = WorkflowValidationResult;
export type RunnerExecutor = Executor;
export type RunnerDestroyableExecutor = RunnerExecutor & {
  destroy?: () => void | Promise<void>;
};
export type RunnerExecutorResult = ExecutorResult;
export type RunnerSolver = Solver;
export type RunnerSolverResult = SolverResult;
export type RunnerEngineEvent = EngineEvent;
export type RunnerEngineCheckpoint = EngineCheckpoint;
export type RunnerRunPlanOptions = RunPlanOptions;
export type RunnerDetectResolver = DetectResolver;

export type RunnerSdkModule = typeof import('../../../ts-sdk/dist/index.js');

export type DryRunSdk = Pick<
  RunnerSdkModule,
  'createSolver' | 'solver' | 'getNodeReadiness' | 'applyRuntimePatches' | 'compileEvmExecution' | 'solana'
>;

export type WorkflowOutputSdk = Pick<RunnerSdkModule, 'evaluateValueRef'>;
