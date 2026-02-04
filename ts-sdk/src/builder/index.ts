/**
 * Builder DSL - Type-safe fluent API for constructing AIS documents
 *
 * @example
 * ```ts
 * import { protocol, pack, workflow, param } from '@owliabot/ais-ts-sdk';
 *
 * const uniswap = protocol('uniswap-v3', '1.0.0')
 *   .description('Uniswap V3 DEX')
 *   .deployment('eip155:1', { router: '0x...' })
 *   .action('swap', { contract: 'router', method: 'swap', params: [param('amount', 'uint256')] })
 *   .build();
 * ```
 */

// Base utilities
export { param, output, type ParamDef, type OutputDef } from './base.js';

// Builders
export { ProtocolBuilder, protocol } from './protocol.js';
export { PackBuilder, pack } from './pack.js';
export { WorkflowBuilder, workflow } from './workflow.js';
