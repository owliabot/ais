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
  type ChainId,
  type Asset,
  type TokenAmount,
  type AISType,

  // Execution
  ExecutionSpecSchema,
  ExecutionBlockSchema,
  type Detect,
  type MappingValue,
  type Mapping,
  type ExecutionSpec,
  type ExecutionBlock,
  type CompositeStep,
  type EvmRead,
  type EvmMultiread,
  type EvmCall,
  type EvmMulticall,
  type Composite,
  type SolanaInstruction,
  type CosmosMessage,
  type BitcoinPsbt,
  type MoveEntry,

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
  type SkillInclude,
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
