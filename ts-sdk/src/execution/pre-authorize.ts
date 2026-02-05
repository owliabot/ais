/**
 * Pre-Authorization Module
 * 
 * Handles token approval flows before main transaction:
 * - approve: Standard ERC20 approve tx
 * - permit: EIP-2612 signature (off-chain)
 * - permit2: Uniswap Permit2 flow
 */

import type { EvmCall, Mapping } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import type { CELContext } from '../cel/evaluator.js';
import type { Evaluator } from '../cel/evaluator.js';
import type { ProtocolSpec } from '../schema/index.js';
import type { TransactionRequest } from './builder.js';
import { encodeFunctionCall, buildFunctionSignature } from './encoder.js';
import { getContractAddress } from '../resolver/reference.js';

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

export interface PreAuthorizeConfig {
  method: 'approve' | 'permit' | 'permit2';
  token: string;
  spender: string;
  amount: string;
}

/**
 * Result of pre-authorization check
 */
export interface PreAuthorizeResult {
  /** Whether authorization is needed */
  needed: boolean;
  /** Approval transaction (for 'approve' method) */
  approveTx?: TransactionRequest;
  /** Permit signature data (for 'permit' and 'permit2' methods) */
  permitData?: PermitData;
  /** Permit2 specific: approval tx for Permit2 contract itself */
  permit2ApproveTx?: TransactionRequest;
}

export interface PermitData {
  /** EIP-712 domain for signing */
  domain: {
    name: string;
    version: string;
    chainId: number;
    verifyingContract: string;
  };
  /** EIP-712 types */
  types: Record<string, Array<{ name: string; type: string }>>;
  /** Message to sign */
  message: Record<string, unknown>;
  /** Primary type name */
  primaryType: string;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════════

/** Standard ERC20 ABI fragments */
const ERC20_APPROVE_ABI = '(address,uint256)';
const ERC20_ALLOWANCE_ABI = '(address,address)';

/** Permit2 contract addresses (same on all EVM chains) */
const PERMIT2_ADDRESS = '0x000000000022D473030F116dDEE9F6B43aC78BA3';

/** Max uint256 for unlimited approvals */
const MAX_UINT256 = BigInt('0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff');

/** EIP-2612 Permit type */
const PERMIT_TYPES = {
  Permit: [
    { name: 'owner', type: 'address' },
    { name: 'spender', type: 'address' },
    { name: 'value', type: 'uint256' },
    { name: 'nonce', type: 'uint256' },
    { name: 'deadline', type: 'uint256' },
  ],
};

/** Permit2 PermitSingle type */
const PERMIT2_TYPES = {
  PermitSingle: [
    { name: 'details', type: 'PermitDetails' },
    { name: 'spender', type: 'address' },
    { name: 'sigDeadline', type: 'uint256' },
  ],
  PermitDetails: [
    { name: 'token', type: 'address' },
    { name: 'amount', type: 'uint160' },
    { name: 'expiration', type: 'uint48' },
    { name: 'nonce', type: 'uint48' },
  ],
};

// ═══════════════════════════════════════════════════════════════════════════════
// Resolution Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Resolve token address from config
 */
function resolveTokenAddress(
  token: string,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  protocol: ProtocolSpec,
  chain: string
): string {
  // Direct address
  if (token.startsWith('0x')) {
    return token;
  }

  // params.token_in.address pattern
  if (token.startsWith('params.')) {
    const key = token.endsWith('.address') ? token : `${token}.address`;
    const value = ctx.variables[key];
    if (typeof value === 'string' && value.startsWith('0x')) {
      return value;
    }
    throw new Error(`Token reference "${token}" did not resolve to address`);
  }

  // contracts.* pattern
  if (token.startsWith('contracts.')) {
    const contractName = token.slice('contracts.'.length);
    const addr = getContractAddress(protocol, chain, contractName);
    if (!addr) {
      throw new Error(`Contract "${contractName}" not found for chain "${chain}"`);
    }
    return addr;
  }

  throw new Error(`Cannot resolve token address from: ${token}`);
}

/**
 * Resolve spender address from config
 */
function resolveSpenderAddress(
  spender: string,
  ctx: ResolverContext,
  protocol: ProtocolSpec,
  chain: string
): string {
  // Direct address
  if (spender.startsWith('0x')) {
    return spender;
  }

  // Contract name from deployments
  const addr = getContractAddress(protocol, chain, spender);
  if (!addr) {
    throw new Error(`Spender contract "${spender}" not found for chain "${chain}"`);
  }
  return addr;
}

/**
 * Resolve amount from config (CEL expression or literal)
 */
function resolveAmount(
  amount: string,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator
): bigint {
  // Literal number
  if (/^-?\d+$/.test(amount)) {
    return BigInt(amount);
  }

  // max or MAX_UINT256
  if (amount.toLowerCase() === 'max' || amount === 'MAX_UINT256') {
    return MAX_UINT256;
  }

  // CEL expression
  try {
    const result = evaluator.evaluate(amount, celCtx);
    if (typeof result === 'number') {
      return BigInt(Math.floor(result));
    }
    if (typeof result === 'bigint') {
      return result;
    }
    if (typeof result === 'string' && /^\d+$/.test(result)) {
      return BigInt(result);
    }
    throw new Error(`Amount expression returned non-numeric: ${result}`);
  } catch (err) {
    throw new Error(
      `Failed to evaluate amount "${amount}": ${err instanceof Error ? err.message : err}`
    );
  }
}

/**
 * Parse chain ID from chain string (e.g., "eip155:1" → 1)
 */
function parseChainId(chain: string): number {
  const match = chain.match(/^eip155:(\d+)$/);
  if (!match) {
    throw new Error(`Unsupported chain format: ${chain}. Only EVM chains supported.`);
  }
  return parseInt(match[1], 10);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Approval Transaction Builders
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Build standard ERC20 approve transaction
 */
function buildApproveTx(
  tokenAddress: string,
  spenderAddress: string,
  amount: bigint,
  chainId: number
): TransactionRequest {
  const signature = buildFunctionSignature('approve', ['address', 'uint256']);
  const data = encodeFunctionCall(signature, ['address', 'uint256'], [spenderAddress, amount]);

  return {
    to: tokenAddress,
    data,
    value: 0n,
    chainId,
    stepId: 'pre_authorize_approve',
    stepDescription: `Approve ${spenderAddress} to spend token`,
  };
}

/**
 * Build ERC20 allowance check call data (for simulation/checking)
 */
export function buildAllowanceCheckData(
  tokenAddress: string,
  ownerAddress: string,
  spenderAddress: string
): { to: string; data: string } {
  const signature = buildFunctionSignature('allowance', ['address', 'address']);
  const data = encodeFunctionCall(signature, ['address', 'address'], [ownerAddress, spenderAddress]);

  return { to: tokenAddress, data };
}

// ═══════════════════════════════════════════════════════════════════════════════
// Permit Data Builders
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Build EIP-2612 permit data for signing
 */
function buildPermitData(
  tokenAddress: string,
  tokenName: string,
  ownerAddress: string,
  spenderAddress: string,
  amount: bigint,
  nonce: bigint,
  deadline: bigint,
  chainId: number
): PermitData {
  return {
    domain: {
      name: tokenName,
      version: '1',
      chainId,
      verifyingContract: tokenAddress,
    },
    types: PERMIT_TYPES,
    primaryType: 'Permit',
    message: {
      owner: ownerAddress,
      spender: spenderAddress,
      value: amount.toString(),
      nonce: nonce.toString(),
      deadline: deadline.toString(),
    },
  };
}

/**
 * Build Permit2 PermitSingle data for signing
 */
function buildPermit2Data(
  tokenAddress: string,
  spenderAddress: string,
  amount: bigint,
  expiration: bigint,
  nonce: bigint,
  sigDeadline: bigint,
  chainId: number
): PermitData {
  return {
    domain: {
      name: 'Permit2',
      version: '1',
      chainId,
      verifyingContract: PERMIT2_ADDRESS,
    },
    types: PERMIT2_TYPES,
    primaryType: 'PermitSingle',
    message: {
      details: {
        token: tokenAddress,
        amount: amount.toString(),
        expiration: expiration.toString(),
        nonce: nonce.toString(),
      },
      spender: spenderAddress,
      sigDeadline: sigDeadline.toString(),
    },
  };
}

// ═══════════════════════════════════════════════════════════════════════════════
// Main Pre-Authorization Logic
// ═══════════════════════════════════════════════════════════════════════════════

export interface PreAuthorizeContext {
  /** Wallet address (owner) */
  walletAddress: string;
  /** Current allowance (optional, for skip logic) */
  currentAllowance?: bigint;
  /** Token name (for EIP-2612 permit) */
  tokenName?: string;
  /** Current nonce (for permit/permit2) */
  nonce?: bigint;
  /** Deadline timestamp (unix seconds) */
  deadline?: bigint;
  /** Permit2 allowance (for permit2 method) */
  permit2Allowance?: bigint;
}

/**
 * Build pre-authorization based on method
 * 
 * @param config - Pre-authorize configuration from spec
 * @param ctx - Resolver context
 * @param celCtx - CEL evaluation context
 * @param evaluator - CEL evaluator
 * @param protocol - Protocol spec
 * @param chain - Chain identifier (e.g., "eip155:1")
 * @param preAuthCtx - Runtime context (wallet, allowances, nonces)
 * @returns Pre-authorization result with transactions/permit data
 */
export function buildPreAuthorize(
  config: PreAuthorizeConfig,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  protocol: ProtocolSpec,
  chain: string,
  preAuthCtx: PreAuthorizeContext
): PreAuthorizeResult {
  const chainId = parseChainId(chain);

  // Resolve addresses and amount
  const tokenAddress = resolveTokenAddress(config.token, ctx, celCtx, evaluator, protocol, chain);
  const spenderAddress = resolveSpenderAddress(config.spender, ctx, protocol, chain);
  const amount = resolveAmount(config.amount, ctx, celCtx, evaluator);

  switch (config.method) {
    case 'approve': {
      // Check if approval is needed
      if (preAuthCtx.currentAllowance !== undefined && preAuthCtx.currentAllowance >= amount) {
        return { needed: false };
      }

      // Build approve transaction
      const approveTx = buildApproveTx(tokenAddress, spenderAddress, amount, chainId);
      return { needed: true, approveTx };
    }

    case 'permit': {
      // Check if already approved (skip permit if so)
      if (preAuthCtx.currentAllowance !== undefined && preAuthCtx.currentAllowance >= amount) {
        return { needed: false };
      }

      // Need token name and nonce for EIP-2612
      if (!preAuthCtx.tokenName) {
        throw new Error('Token name required for EIP-2612 permit');
      }

      const nonce = preAuthCtx.nonce ?? 0n;
      const deadline = preAuthCtx.deadline ?? BigInt(Math.floor(Date.now() / 1000) + 3600); // 1 hour

      const permitData = buildPermitData(
        tokenAddress,
        preAuthCtx.tokenName,
        preAuthCtx.walletAddress,
        spenderAddress,
        amount,
        nonce,
        deadline,
        chainId
      );

      return { needed: true, permitData };
    }

    case 'permit2': {
      // Permit2 flow:
      // 1. Check if token approved to Permit2 contract
      // 2. If not, create approve tx for Permit2
      // 3. Create Permit2 signature for spender

      const result: PreAuthorizeResult = { needed: true };

      // Check Permit2 approval (token → Permit2 contract)
      if (preAuthCtx.permit2Allowance === undefined || preAuthCtx.permit2Allowance < amount) {
        // Need to approve Permit2 contract first
        result.permit2ApproveTx = buildApproveTx(tokenAddress, PERMIT2_ADDRESS, MAX_UINT256, chainId);
        result.permit2ApproveTx.stepId = 'pre_authorize_permit2_approve';
        result.permit2ApproveTx.stepDescription = 'Approve Permit2 contract to spend token';
      }

      // Build Permit2 signature data
      const nonce = preAuthCtx.nonce ?? 0n;
      const sigDeadline = preAuthCtx.deadline ?? BigInt(Math.floor(Date.now() / 1000) + 3600);
      // Default expiration: 30 days
      const expiration = BigInt(Math.floor(Date.now() / 1000) + 30 * 24 * 3600);

      result.permitData = buildPermit2Data(
        tokenAddress,
        spenderAddress,
        amount,
        expiration,
        nonce,
        sigDeadline,
        chainId
      );

      return result;
    }

    default:
      throw new Error(`Unknown pre_authorize method: ${config.method}`);
  }
}

/**
 * Get required queries for pre-authorization
 * Returns query specs that should be executed to get allowance/nonce data
 */
export function getPreAuthorizeQueries(
  config: PreAuthorizeConfig,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  protocol: ProtocolSpec,
  chain: string,
  walletAddress: string
): Array<{ id: string; to: string; data: string; description: string }> {
  const tokenAddress = resolveTokenAddress(config.token, ctx, celCtx, evaluator, protocol, chain);
  const spenderAddress = resolveSpenderAddress(config.spender, ctx, protocol, chain);
  
  const queries: Array<{ id: string; to: string; data: string; description: string }> = [];

  // Always need current allowance check
  const allowanceCheck = buildAllowanceCheckData(tokenAddress, walletAddress, spenderAddress);
  queries.push({
    id: 'pre_auth_allowance',
    ...allowanceCheck,
    description: 'Check current token allowance',
  });

  // For permit2, also check Permit2 allowance
  if (config.method === 'permit2') {
    const permit2AllowanceCheck = buildAllowanceCheckData(tokenAddress, walletAddress, PERMIT2_ADDRESS);
    queries.push({
      id: 'pre_auth_permit2_allowance',
      ...permit2AllowanceCheck,
      description: 'Check Permit2 contract allowance',
    });
  }

  return queries;
}

// Export Permit2 address for external use
export { PERMIT2_ADDRESS, MAX_UINT256 };
