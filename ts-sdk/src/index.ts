/**
 * @ais-protocol/sdk
 * TypeScript SDK for parsing and validating AIS (Agent Interaction Specification) files
 */

// Types
export type {
  AISDocument,
  AISDocumentType,
  ProtocolSpec,
  ProtocolMeta,
  Query,
  QueryInput,
  QueryOutput,
  Action,
  ActionInput,
  ActionOutput,
  ConsistencyCheck,
  CustomType,
  Pack,
  PackMeta,
  ProtocolRef,
  PackConstraints,
  AmountConstraint,
  SlippageConstraint,
  Workflow,
  WorkflowMeta,
  WorkflowInput,
  WorkflowStep,
  AnyAISDocument,
  Asset,
  TokenAmount,
} from './types.js';

// Schemas
export {
  ProtocolSpecSchema,
  PackSchema,
  WorkflowSchema,
  AISDocumentSchema,
  AssetSchema,
  TokenAmountSchema,
} from './schema.js';

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
  createContext,
  registerProtocol,
  resolveProtocolRef,
  resolveAction,
  resolveQuery,
  expandPack,
  hasExpressions,
  extractExpressions,
  resolveExpression,
  resolveExpressionString,
  setVariable,
  setQueryResult,
  type ResolverContext,
} from './resolver.js';
