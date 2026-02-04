/**
 * AIS Protocol SDK - Type Definitions
 * Based on AIS-1 (Core), AIS-2 (Execution), AIS-3 (Registry)
 */

// =============================================================================
// Common Types
// =============================================================================

export type AISDocumentType = 'protocol' | 'pack' | 'workflow';

export interface AISDocument {
  ais_version: string;
  type: AISDocumentType;
}

// =============================================================================
// Protocol Spec Types (.ais.yaml)
// =============================================================================

export interface ProtocolMeta {
  name: string;
  version: string;
  chain_id: number;
  description?: string;
  addresses: Record<string, string>;
}

export interface QueryInput {
  name: string;
  type: string;
  description?: string;
}

export interface QueryOutput {
  name: string;
  type: string;
  path?: string;
  description?: string;
}

export interface Query {
  name: string;
  contract: string;
  method: string;
  inputs?: QueryInput[];
  outputs: QueryOutput[];
  description?: string;
}

export interface ActionInput {
  name: string;
  type: string;
  description?: string;
  default?: unknown;
  calculated_from?: string;
}

export interface ActionOutput {
  name: string;
  type: string;
  path?: string;
}

export interface ConsistencyCheck {
  condition: string;
  message: string;
}

export interface Action {
  name: string;
  contract: string;
  method: string;
  inputs: ActionInput[];
  outputs?: ActionOutput[];
  requires_queries?: string[];
  calculated_fields?: Record<string, string>;
  consistency?: ConsistencyCheck[];
  description?: string;
}

export interface CustomType {
  name: string;
  base: string;
  fields?: Record<string, string>;
  description?: string;
}

export interface ProtocolSpec extends AISDocument {
  type: 'protocol';
  protocol: ProtocolMeta;
  queries?: Query[];
  actions: Action[];
  types?: CustomType[];
}

// =============================================================================
// Pack Types (.ais-pack.yaml)
// =============================================================================

export interface PackMeta {
  name: string;
  version: string;
  description?: string;
  maintainer?: string;
}

export interface ProtocolRef {
  protocol: string;
  version: string;
  source?: string;
  actions?: string[];
}

export interface AmountConstraint {
  max_usd?: number;
  max_percentage_of_balance?: number;
}

export interface SlippageConstraint {
  max_bps: number;
}

export interface PackConstraints {
  tokens?: {
    allowlist?: string[];
    blocklist?: string[];
  };
  amounts?: AmountConstraint;
  slippage?: SlippageConstraint;
  require_simulation?: boolean;
}

export interface Pack extends AISDocument {
  type: 'pack';
  pack: PackMeta;
  protocols: ProtocolRef[];
  constraints?: PackConstraints;
}

// =============================================================================
// Workflow Types (.ais-flow.yaml)
// =============================================================================

export interface WorkflowMeta {
  name: string;
  version: string;
  description?: string;
}

export interface WorkflowInput {
  name: string;
  type: string;
  description?: string;
  default?: unknown;
}

export interface WorkflowStep {
  id: string;
  uses: string;
  with: Record<string, unknown>;
  outputs?: Record<string, string>;
  condition?: string;
}

export interface Workflow extends AISDocument {
  type: 'workflow';
  workflow: WorkflowMeta;
  inputs: WorkflowInput[];
  steps: WorkflowStep[];
}

// =============================================================================
// Union Type
// =============================================================================

export type AnyAISDocument = ProtocolSpec | Pack | Workflow;

// =============================================================================
// Asset Types (from AIS-1 composite types)
// =============================================================================

export interface Asset {
  chain_id: number;
  address: string;
  symbol?: string;
  decimals?: number;
}

export interface TokenAmount {
  asset: Asset | string;
  amount: string;
  human_readable?: string;
}
