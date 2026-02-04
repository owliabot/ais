/**
 * @owliabot/ais-ts-sdk
 * TypeScript SDK for parsing and validating AIS (Agent Interaction Specification) files
 */

// Schema types and validators
export {
  // Common
  AssetSchema,
  TokenAmountSchema,
  type Asset,
  type TokenAmount,

  // Protocol
  ProtocolSpecSchema,
  type ProtocolSpec,
  type Meta,
  type Deployment,
  type Param,
  type Query,
  type Action,
  type Risk,
  type Constraint,
  type CalculatedField,
  type AssetMapping,
  type TestVector,

  // Pack
  PackSchema,
  type Pack,
  type Policy,
  type HardConstraints,
  type TokenPolicy,
  type Providers,
  type SkillOverride,

  // Workflow
  WorkflowSchema,
  type Workflow,
  type WorkflowMeta,
  type WorkflowInput,
  type WorkflowNode,
  type WorkflowPolicy,
  type PackRef,

  // Union
  AISDocumentSchema,
  type AnyAISDocument,
  type AISSchemaType,
} from './schema/index.js';

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
  setVariable,
  setQueryResult,
  type ResolverContext,

  // References
  registerProtocol,
  parseSkillRef,
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
} from './validator/index.js';

// Loader
export {
  loadFile,
  loadProtocol,
  loadPack,
  loadWorkflow,
  loadDirectory,
  loadDirectoryAsContext,
  type LoadResult,
  type LoadError,
  type DirectoryLoadResult,
} from './loader.js';

// Execution
export {
  buildTransaction,
  buildQuery,
  buildWorkflowTransactions,
  encodeFunctionCall,
  encodeFunctionSelector,
  encodeValue,
  buildFunctionSignature,
  keccak256,
  type TransactionRequest,
  type BuildOptions,
  type BuildResult,
  type BuildError,
  type BuildOutput,
} from './execution/index.js';

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
