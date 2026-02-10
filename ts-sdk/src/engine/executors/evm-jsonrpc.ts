import type { Executor, ExecutorResult } from '../types.js';
import type { ResolverContext } from '../../resolver/index.js';
import { evaluateValueRef } from '../../resolver/index.js';
import type { DetectResolver } from '../../resolver/value-ref.js';
import type { ExecutionPlanNode } from '../../execution/index.js';
import { compileEvmExecution, compileEvmExecutionAsync, type CompiledEvmRequest, type CompiledEvmGetBalanceRequest } from '../../execution/index.js';
import { decodeJsonAbiFunctionResult } from '../../execution/index.js';
import type { RuntimePatch } from '../patch.js';

export interface JsonRpcTransport {
  request<T = unknown>(method: string, params: unknown[]): Promise<T>;
}

export interface EvmTxRequest {
  chainId: number;
  to: string;
  data: string;
  value: bigint;
}

export interface EvmSigner {
  signTransaction(tx: EvmTxRequest): Promise<string>;
}

export interface EvmJsonRpcExecutorOptions {
  transport: JsonRpcTransport;
  signer?: EvmSigner;
  /**
   * When true, `evm_call` will poll `eth_getTransactionReceipt` until a receipt
   * is available (or `receipt_poll.max_attempts` is exceeded).
   */
  wait_for_receipt?: boolean;
  receipt_poll?: { interval_ms?: number; max_attempts?: number };
}

export class EvmJsonRpcExecutor implements Executor {
  private readonly transport: JsonRpcTransport;
  private readonly signer?: EvmSigner;
  private readonly waitForReceipt: boolean;
  private readonly pollIntervalMs: number;
  private readonly pollMaxAttempts: number;

  constructor(options: EvmJsonRpcExecutorOptions) {
    this.transport = options.transport;
    this.signer = options.signer;
    this.waitForReceipt = options.wait_for_receipt ?? false;
    this.pollIntervalMs = options.receipt_poll?.interval_ms ?? 1000;
    this.pollMaxAttempts = options.receipt_poll?.max_attempts ?? 60;
  }

  supports(node: ExecutionPlanNode): boolean {
    if (!node.chain.startsWith('eip155:')) return false;
    return node.execution.type === 'evm_read' || node.execution.type === 'evm_call' || node.execution.type === 'evm_get_balance';
  }

  async execute(
    node: ExecutionPlanNode,
    ctx: ResolverContext,
    options?: { resolved_params?: Record<string, unknown>; detect?: DetectResolver }
  ): Promise<ExecutorResult> {
    if (!this.supports(node)) {
      return { need_user_confirm: { reason: `Unsupported chain for EVM JSON-RPC executor: ${node.chain}` } };
    }

    const resolvedParams = options?.resolved_params ?? resolveNodeParams(node, ctx);
    const compiled = options?.detect
      ? await compileEvmExecutionAsync(node.execution, ctx, { chain: node.chain, params: resolvedParams, detect: options.detect })
      : compileEvmExecution(node.execution, ctx, { chain: node.chain, params: resolvedParams });

    if (compiled.kind === 'evm_get_balance') {
      return await this.executeGetBalance(node, compiled);
    }

    if (compiled.kind === 'evm_read') {
      return await this.executeRead(node, ctx, compiled);
    }

    return await this.executeCall(node, ctx, compiled);
  }

  private async executeRead(
    node: ExecutionPlanNode,
    _ctx: ResolverContext,
    compiled: CompiledEvmRequest
  ): Promise<ExecutorResult> {
    if (compiled.kind !== 'evm_read') throw new Error(`Internal error: executeRead called with kind=${compiled.kind}`);
    const call = { to: compiled.to, data: compiled.data };
    const returnData = await this.transport.request<string>('eth_call', [call, 'latest']);
    const outputs = decodeJsonAbiFunctionResult(compiled.abi, returnData);
    const patches = applyWritesToPatches(node, outputs);
    return { outputs, patches, telemetry: { method: 'eth_call', to: compiled.to } };
  }

  private async executeGetBalance(
    node: ExecutionPlanNode,
    compiled: CompiledEvmGetBalanceRequest
  ): Promise<ExecutorResult> {
    const hex = await this.transport.request<string>('eth_getBalance', [compiled.address, compiled.blockTag]);
    const balance = hexQuantityToBigInt(hex);
    const outputs = { balance };
    const patches = applyWritesToPatches(node, outputs);
    return { outputs, patches, telemetry: { method: 'eth_getBalance', address: compiled.address } };
  }

  private async executeCall(
    node: ExecutionPlanNode,
    _ctx: ResolverContext,
    compiled: CompiledEvmRequest
  ): Promise<ExecutorResult> {
    if (compiled.kind !== 'evm_call') throw new Error(`Internal error: executeCall called with kind=${compiled.kind}`);
    const tx: EvmTxRequest = {
      chainId: compiled.chainId,
      to: compiled.to,
      data: compiled.data,
      value: compiled.value,
    };

    if (!this.signer) {
      return {
        need_user_confirm: {
          reason: 'Missing signer: provide EvmSigner to sign and broadcast evm_call',
          details: { tx },
        },
      };
    }

    const raw = await this.signer.signTransaction(tx);
    const txHash = await this.transport.request<string>('eth_sendRawTransaction', [raw]);

    let receipt: unknown | null = null;
    if (this.waitForReceipt) {
      receipt = await this.pollReceipt(txHash);
    }

    const outputs = receipt ? { tx_hash: txHash, receipt } : { tx_hash: txHash };
    const patches = applyWritesToPatches(node, outputs);
    return { outputs, patches, telemetry: { method: 'eth_sendRawTransaction', tx_hash: txHash } };
  }

  private async pollReceipt(txHash: string): Promise<unknown> {
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    for (let attempt = 0; attempt < this.pollMaxAttempts; attempt++) {
      const receipt = await this.transport.request<unknown>('eth_getTransactionReceipt', [txHash]);
      if (receipt) return receipt;
      await sleep(this.pollIntervalMs);
    }
    throw new Error(`Receipt not found after ${this.pollMaxAttempts} attempts`);
  }
}

function resolveNodeParams(node: ExecutionPlanNode, ctx: ResolverContext): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (!node.bindings?.params) return out;
  for (const [k, v] of Object.entries(node.bindings.params)) {
    out[k] = evaluateValueRef(v, ctx);
  }
  return out;
}

function applyWritesToPatches(node: ExecutionPlanNode, outputs: Record<string, unknown>): RuntimePatch[] {
  if (!node.writes || node.writes.length === 0) {
    return [{ op: 'merge', path: `nodes.${node.id}.outputs`, value: outputs }];
  }
  return node.writes.map((w) => ({
    op: w.mode === 'merge' ? 'merge' : 'set',
    path: w.path,
    value: outputs,
  }));
}

function hexQuantityToBigInt(hex: string): bigint {
  if (typeof hex !== 'string') throw new Error(`Invalid hex quantity: expected string, got ${typeof hex}`);
  if (!hex.startsWith('0x')) throw new Error('Invalid hex quantity: missing 0x prefix');
  // JSON-RPC quantities are hex without leading zeros (except 0x0).
  return BigInt(hex);
}
