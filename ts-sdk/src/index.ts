/**
 * @ais-protocol/sdk
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
  type ProtocolMeta,
  type Query,
  type QueryInput,
  type QueryOutput,
  type Action,
  type ActionInput,
  type ActionOutput,
  type ConsistencyCheck,
  type CustomType,

  // Pack
  PackSchema,
  type Pack,
  type PackMeta,
  type ProtocolRef,
  type PackConstraints,
  type AmountConstraint,
  type SlippageConstraint,

  // Workflow
  WorkflowSchema,
  type Workflow,
  type WorkflowMeta,
  type WorkflowInput,
  type WorkflowStep,

  // Union
  AISDocumentSchema,
  type AnyAISDocument,
  type AISDocumentType,
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
  resolveProtocolRef,
  resolveAction,
  resolveQuery,
  expandPack,

  // Expressions
  hasExpressions,
  extractExpressions,
  resolveExpression,
  resolveExpressionString,
} from './resolver/index.js';
