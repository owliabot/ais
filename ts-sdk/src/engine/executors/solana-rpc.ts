import type { Executor, ExecutorResult } from '../types.js';
import type { ExecutionPlanNode } from '../../execution/index.js';
import type { ResolverContext } from '../../resolver/index.js';
import { evaluateValueRef, evaluateValueRefAsync } from '../../resolver/index.js';
import type { DetectResolver } from '../../resolver/value-ref.js';
import { solana } from '../../execution/index.js';
import { isCoreExecutionSpec, type SolanaRead } from '../../schema/index.js';
import { Buffer } from 'node:buffer';
import {
  AddressLookupTableAccount,
  ComputeBudgetProgram,
  PublicKey,
  Transaction,
  TransactionMessage,
  VersionedTransaction,
  type Commitment,
  type TransactionConfirmationStrategy,
} from '@solana/web3.js';

export interface SolanaSigner {
  publicKey: PublicKey;
  signTransaction(
    tx: Transaction | VersionedTransaction
  ): Promise<Transaction | VersionedTransaction> | Transaction | VersionedTransaction;
}

export interface SolanaRpcConnectionLike {
  getLatestBlockhash(commitment?: Commitment): Promise<{ blockhash: string; lastValidBlockHeight: number }>;
  sendRawTransaction(
    raw: Uint8Array,
    options?: { skipPreflight?: boolean; maxRetries?: number; preflightCommitment?: Commitment }
  ): Promise<string>;
  confirmTransaction(
    strategy: TransactionConfirmationStrategy,
    commitment?: Commitment
  ): Promise<{ value: { err: unknown | null } }>;
  getBalance(pubkey: PublicKey, commitment?: Commitment): Promise<number>;
  getTokenAccountBalance(pubkey: PublicKey, commitment?: Commitment): Promise<unknown>;
  getAccountInfo(pubkey: PublicKey, commitment?: Commitment): Promise<unknown | null>;
  getSignatureStatuses(
    signatures: string[],
    config?: { searchTransactionHistory?: boolean }
  ): Promise<{ value: Array<unknown | null> }>;
  getAddressLookupTable?(
    address: PublicKey,
    commitment?: Commitment
  ): Promise<{ value: AddressLookupTableAccount | null }>;
}

export interface SolanaRpcExecutorOptions {
  connection: SolanaRpcConnectionLike;
  signer?: SolanaSigner;
  /**
   * If not provided, defaults to `signer.publicKey`.
   * When signer is missing, the executor returns `need_user_confirm` with an unsigned tx.
   */
  fee_payer?: PublicKey;
  commitment?: Commitment;
  send_options?: { skipPreflight?: boolean; maxRetries?: number; preflightCommitment?: Commitment };
  wait_for_confirmation?: boolean;
  compiler_registry?: solana.SolanaInstructionCompilerRegistry;
}

export class SolanaRpcExecutor implements Executor {
  private readonly connection: SolanaRpcConnectionLike;
  private readonly signer?: SolanaSigner;
  private readonly feePayer?: PublicKey;
  private readonly commitment?: Commitment;
  private readonly sendOptions?: SolanaRpcExecutorOptions['send_options'];
  private readonly waitForConfirmation: boolean;
  private readonly compilerRegistry?: solana.SolanaInstructionCompilerRegistry;

  constructor(options: SolanaRpcExecutorOptions) {
    this.connection = options.connection;
    this.signer = options.signer;
    this.feePayer = options.fee_payer;
    this.commitment = options.commitment;
    this.sendOptions = options.send_options;
    this.waitForConfirmation = options.wait_for_confirmation ?? true;
    this.compilerRegistry = options.compiler_registry;
  }

  supports(node: ExecutionPlanNode): boolean {
    if (!node.chain.startsWith('solana:')) return false;
    return (
      isCoreExecutionSpec(node.execution) &&
      (node.execution.type === 'solana_instruction' || node.execution.type === 'solana_read')
    );
  }

  async execute(
    node: ExecutionPlanNode,
    ctx: ResolverContext,
    options?: { resolved_params?: Record<string, unknown>; detect?: DetectResolver }
  ): Promise<ExecutorResult> {
    if (!this.supports(node)) {
      return { need_user_confirm: { reason: `Unsupported node for SolanaRpcExecutor: ${node.chain}/${node.execution.type}` } };
    }

    const exec = node.execution;
    if (!isCoreExecutionSpec(exec)) {
      return { need_user_confirm: { reason: `Unsupported node for SolanaRpcExecutor: ${node.chain}/${exec.type}` } };
    }

    if (exec.type === 'solana_read') {
      return await this.executeRead(exec, node, ctx, options?.resolved_params, options?.detect);
    }

    if (exec.type !== 'solana_instruction') {
      return { need_user_confirm: { reason: `Unsupported node for SolanaRpcExecutor: ${node.chain}/${exec.type}` } };
    }

    const resolvedParams = options?.resolved_params ?? resolveNodeParams(node, ctx);
    const compiled = options?.detect
      ? await solana.compileSolanaInstructionAsync(exec, ctx, {
          chain: node.chain,
          params: resolvedParams,
          compiler_registry: this.compilerRegistry,
          detect: options.detect,
        })
      : solana.compileSolanaInstruction(exec, ctx, {
          chain: node.chain,
          params: resolvedParams,
          compiler_registry: this.compilerRegistry,
        });

    const feePayer = this.feePayer ?? this.signer?.publicKey;
    if (!feePayer) {
      return {
        need_user_confirm: {
          reason: 'Missing fee payer (no signer provided and fee_payer not set)',
          details: { chain: node.chain, program: compiled.programId.toBase58(), instruction: compiled.instruction },
        },
      };
    }

    const { blockhash, lastValidBlockHeight } = await this.connection.getLatestBlockhash(this.commitment);
    const instructions = [
      ...(compiled.computeUnits ? [ComputeBudgetProgram.setComputeUnitLimit({ units: compiled.computeUnits })] : []),
      compiled.tx,
    ];

    const needsV0 = (compiled.lookupTables?.length ?? 0) > 0;
    const unsigned = needsV0
      ? await buildV0Transaction(this.connection, feePayer, blockhash, instructions, compiled.lookupTables ?? [], this.commitment)
      : buildLegacyTransaction(feePayer, blockhash, instructions);

    if (!this.signer) {
      return {
        need_user_confirm: {
          reason: 'Missing signer: provide SolanaSigner to sign and broadcast solana_instruction',
          details: { tx: describeUnsignedTx(unsigned), chain: node.chain },
        },
      };
    }

    const signed = await Promise.resolve(this.signer.signTransaction(unsigned));
    const raw = serializeSignedTx(signed);
    const signature = await this.connection.sendRawTransaction(raw, this.sendOptions);

    let confirmation: unknown | undefined;
    if (this.waitForConfirmation) {
      const strategy: TransactionConfirmationStrategy = { signature, blockhash, lastValidBlockHeight };
      const result = await this.connection.confirmTransaction(strategy, this.commitment);
      if (result.value.err) {
        throw new Error(`Solana transaction failed: ${JSON.stringify(result.value.err)}`);
      }
      confirmation = result;
    }

    const outputs = confirmation ? { signature, confirmation } : { signature };
    return {
      outputs,
      patches: applyWritesToPatches(node, outputs),
      telemetry: {
        chain: node.chain,
        program: compiled.programId.toBase58(),
        instruction: compiled.instruction,
        signature,
      },
    };
  }

  private async executeRead(
    exec: SolanaRead,
    node: ExecutionPlanNode,
    ctx: ResolverContext,
    resolvedParamsOverride?: Record<string, unknown>,
    detect?: DetectResolver
  ): Promise<ExecutorResult> {
    const resolvedParams = resolvedParamsOverride ?? resolveNodeParams(node, ctx);
    const params = exec.params
      ? detect
        ? await evaluateValueRefAsync(exec.params, ctx, {
            root_overrides: { params: resolvedParams },
            detect,
          })
        : evaluateValueRef(exec.params, ctx, { root_overrides: { params: resolvedParams } })
      : resolvedParams;
    const method = exec.method;

    let outputs: Record<string, unknown>;

    if (method === 'getBalance') {
      const pubkey = parsePubkeyFromParams(params, 'params');
      const lamports = await this.connection.getBalance(pubkey, this.commitment);
      outputs = { lamports: BigInt(lamports) };
    } else if (method === 'getTokenAccountBalance') {
      const pubkey = parsePubkeyFromParams(params, 'params');
      const res = await this.connection.getTokenAccountBalance(pubkey, this.commitment);
      outputs = { result: res };
    } else if (method === 'getSignatureStatuses') {
      const { signatures, searchTransactionHistory } = parseSignatureStatusesParams(params);
      const res = await this.connection.getSignatureStatuses(signatures, { searchTransactionHistory });
      outputs = { statuses: res.value };
    } else if (method === 'getAccountInfo') {
      const pubkey = parsePubkeyFromParams(params, 'params');
      const info = await this.connection.getAccountInfo(pubkey, this.commitment);
      outputs = normalizeAccountInfo(info);
    } else {
      throw new Error(`Unsupported solana_read method: ${method}`);
    }

    return {
      outputs,
      patches: applyWritesToPatches(node, outputs),
      telemetry: { chain: node.chain, method },
    };
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

function buildLegacyTransaction(
  feePayer: PublicKey,
  blockhash: string,
  instructions: Transaction['instructions']
): Transaction {
  const tx = new Transaction();
  tx.feePayer = feePayer;
  tx.recentBlockhash = blockhash;
  tx.add(...instructions);
  return tx;
}

async function buildV0Transaction(
  connection: SolanaRpcConnectionLike,
  feePayer: PublicKey,
  blockhash: string,
  instructions: Transaction['instructions'],
  lookupTableAddresses: string[],
  commitment?: Commitment
): Promise<VersionedTransaction> {
  if (!connection.getAddressLookupTable) {
    throw new Error('Connection does not support getAddressLookupTable() but lookup_tables were provided');
  }

  const lookupTables: AddressLookupTableAccount[] = [];
  for (const addr of lookupTableAddresses) {
    const pk = new PublicKey(addr);
    const r = await connection.getAddressLookupTable(pk, commitment);
    if (!r.value) throw new Error(`Lookup table not found: ${addr}`);
    lookupTables.push(r.value);
  }

  const message = new TransactionMessage({
    payerKey: feePayer,
    recentBlockhash: blockhash,
    instructions,
  }).compileToV0Message(lookupTables);

  return new VersionedTransaction(message);
}

function serializeSignedTx(tx: Transaction | VersionedTransaction): Uint8Array {
  if (tx instanceof Transaction) return tx.serialize();
  return tx.serialize();
}

function describeUnsignedTx(tx: Transaction | VersionedTransaction): { kind: 'legacy' | 'v0'; bytes_base64: string } {
  const raw = tx instanceof Transaction ? tx.serialize({ requireAllSignatures: false, verifySignatures: false }) : tx.serialize();
  return { kind: tx instanceof Transaction ? 'legacy' : 'v0', bytes_base64: Buffer.from(raw).toString('base64') };
}

function applyWritesToPatches(node: ExecutionPlanNode, outputs: Record<string, unknown>) {
  if (!node.writes || node.writes.length === 0) {
    return [{ op: 'merge' as const, path: `nodes.${node.id}.outputs`, value: outputs }];
  }
  return node.writes.map((w) => ({
    op: w.mode === 'merge' ? ('merge' as const) : ('set' as const),
    path: w.path,
    value: outputs,
  }));
}

function parsePubkeyFromParams(params: unknown, field: string): PublicKey {
  if (typeof params === 'string') return new PublicKey(params);
  if (params && typeof params === 'object' && !Array.isArray(params)) {
    const o = params as Record<string, unknown>;
    const v = o.address ?? o.pubkey;
    if (typeof v === 'string') return new PublicKey(v);
  }
  throw new Error(`${field} must be a base58 string or { address } or { pubkey }`);
}

function parseSignatureStatusesParams(params: unknown): { signatures: string[]; searchTransactionHistory?: boolean } {
  if (Array.isArray(params) && params.every((x) => typeof x === 'string')) {
    return { signatures: params as string[] };
  }
  if (params && typeof params === 'object' && !Array.isArray(params)) {
    const o = params as Record<string, unknown>;
    const signatures = o.signatures;
    if (!Array.isArray(signatures) || !signatures.every((x) => typeof x === 'string')) {
      throw new Error('params.signatures must be string[]');
    }
    const sth = o.searchTransactionHistory;
    if (sth !== undefined && typeof sth !== 'boolean') throw new Error('params.searchTransactionHistory must be boolean');
    return { signatures: signatures as string[], searchTransactionHistory: sth as boolean | undefined };
  }
  throw new Error('params must be string[] or { signatures: string[], searchTransactionHistory?: boolean }');
}

function normalizeAccountInfo(info: unknown | null): Record<string, unknown> {
  if (!info) return { exists: false };
  const i = info as any;
  const lamports = typeof i.lamports === 'number' ? BigInt(i.lamports) : i.lamports;
  const owner = i.owner?.toBase58 ? i.owner.toBase58() : i.owner;
  const executable = Boolean(i.executable);
  const rentEpoch = typeof i.rentEpoch === 'number' ? BigInt(i.rentEpoch) : i.rentEpoch;
  const dataBytes: Uint8Array | undefined = i.data instanceof Uint8Array ? i.data : undefined;
  const data_base64 = dataBytes ? Buffer.from(dataBytes).toString('base64') : undefined;
  return {
    exists: true,
    lamports,
    owner,
    executable,
    rentEpoch,
    data_base64,
    data_len: dataBytes ? dataBytes.length : undefined,
  };
}
