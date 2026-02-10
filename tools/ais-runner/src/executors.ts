type Executor = {
  supports(node: any): boolean;
  execute(node: any, ctx: any, options?: any): Promise<any> | any;
  destroy?: () => void | Promise<void>;
};

import { readFile } from 'node:fs/promises';
import { requireFromTsSdk } from './deps.js';

export class ChainBoundExecutor implements Executor {
  constructor(
    private readonly chain: string,
    private readonly inner: Executor,
    private readonly cleanup?: () => void | Promise<void>
  ) {}

  supports(node: any): boolean {
    return node?.chain === this.chain && this.inner.supports(node);
  }

  execute(node: any, ctx: any, options?: any): Promise<any> | any {
    return this.inner.execute(node, ctx, options);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
    await this.cleanup?.();
  }
}

export type RunnerExecutorsConfig = {
  chains?: Record<
    string,
    {
      rpc_url?: string;
      wait_for_receipt?: boolean;
      receipt_poll?: { interval_ms?: number; max_attempts?: number };
      commitment?: string;
      wait_for_confirmation?: boolean;
      send_options?: { skipPreflight?: boolean; maxRetries?: number; preflightCommitment?: string };
      signer?: Record<string, unknown>;
    }
  >;
};

export async function createExecutorsFromConfig(
  sdk: any,
  config: RunnerExecutorsConfig | null,
  options: { allow_broadcast: boolean }
): Promise<Executor[]> {
  const chains = config?.chains ?? {};
  const out: Executor[] = [];

  for (const [chain, cfg] of Object.entries(chains)) {
    const rpcUrl = String(cfg?.rpc_url ?? '').trim();
    if (!rpcUrl) continue;

    if (chain.startsWith('eip155:')) {
      const { provider, transport } = createEthersTransport(rpcUrl);
      const signer = options.allow_broadcast ? createEvmSignerFromConfig(cfg?.signer, provider) : undefined;
      const ex = new sdk.EvmJsonRpcExecutor({
        transport,
        signer,
        wait_for_receipt: Boolean(cfg?.wait_for_receipt),
        receipt_poll: {
          interval_ms: cfg?.receipt_poll?.interval_ms,
          max_attempts: cfg?.receipt_poll?.max_attempts,
        },
      });
      out.push(new ChainBoundExecutor(chain, ex, () => provider.destroy?.()));
      continue;
    }

    if (chain.startsWith('solana:')) {
      const connection = createSolanaConnection(rpcUrl, cfg?.commitment);
      const signer = options.allow_broadcast ? await createSolanaSignerFromConfig(cfg?.signer) : undefined;
      const ex = new sdk.SolanaRpcExecutor({
        connection,
        signer,
        commitment: cfg?.commitment,
        wait_for_confirmation: cfg?.wait_for_confirmation,
        send_options: cfg?.send_options,
      });
      out.push(new ChainBoundExecutor(chain, ex));
      continue;
    }
  }

  return out;
}

function createEthersTransport(rpcUrl: string): { provider: any; transport: { request<T>(m: string, p: unknown[]): Promise<T> } } {
  const ethers = requireFromTsSdk('ethers');
  const provider = new ethers.JsonRpcProvider(rpcUrl);
  const transport = {
    async request<T = unknown>(method: string, params: unknown[]): Promise<T> {
      return (await provider.send(method, params)) as T;
    },
  };
  return { provider, transport };
}

function createEvmSignerFromConfig(signerCfg: Record<string, unknown> | undefined, provider: any): any | undefined {
  if (!signerCfg || typeof signerCfg !== 'object') return undefined;
  const type = String((signerCfg as any).type ?? '');
  if (type !== 'evm_private_key') return undefined;

  const envName = (signerCfg as any).private_key_env ? String((signerCfg as any).private_key_env) : '';
  const pk = envName ? (globalThis as any).process?.env?.[envName] : (signerCfg as any).private_key;
  if (!pk) return undefined;

  const ethers = requireFromTsSdk('ethers');
  const priv = normalizeHexPrivateKey(String(pk));
  const wallet = new ethers.Wallet(priv, provider);

  return new EthersBackedEvmSigner(ethers, wallet, provider);
}

class EthersBackedEvmSigner {
  constructor(private readonly ethers: any, private readonly wallet: any, private readonly provider: any) {}

  async signTransaction(tx: { chainId: number; to: string; data: string; value: bigint }): Promise<string> {
    const from = await this.wallet.getAddress();
    const nonce = await this.provider.getTransactionCount(from, 'pending');

    const base: any = {
      chainId: tx.chainId,
      to: tx.to,
      data: tx.data,
      value: tx.value,
      nonce,
    };

    const gasLimit = await this.provider.estimateGas({ ...base, from });
    base.gasLimit = gasLimit;

    const fee = await this.provider.getFeeData();
    if (fee && fee.maxFeePerGas != null && fee.maxPriorityFeePerGas != null) {
      base.maxFeePerGas = fee.maxFeePerGas;
      base.maxPriorityFeePerGas = fee.maxPriorityFeePerGas;
    } else {
      base.gasPrice = await this.provider.getGasPrice();
    }

    return await this.wallet.signTransaction(base);
  }
}

function createSolanaConnection(rpcUrl: string, commitment: string | undefined): any {
  const web3 = requireFromTsSdk('@solana/web3.js');
  return new web3.Connection(rpcUrl, commitment ?? 'confirmed');
}

async function createSolanaSignerFromConfig(signerCfg: Record<string, unknown> | undefined): Promise<any | undefined> {
  if (!signerCfg || typeof signerCfg !== 'object') return undefined;
  const type = String((signerCfg as any).type ?? '');
  if (type !== 'solana_keypair_file') return undefined;
  const path = (signerCfg as any).keypair_path ? String((signerCfg as any).keypair_path) : '';
  if (!path) return undefined;
  const expanded = expandTilde(path);
  const raw = await readFile(expanded, 'utf-8');
  const arr = JSON.parse(raw);
  if (!Array.isArray(arr) || arr.some((x) => typeof x !== 'number')) {
    throw new Error('Invalid Solana keypair file: expected JSON number array');
  }

  const web3 = requireFromTsSdk('@solana/web3.js');
  const secret = Uint8Array.from(arr as number[]);
  const keypair = web3.Keypair.fromSecretKey(secret);

  return {
    publicKey: keypair.publicKey,
    signTransaction(tx: any) {
      // Support both Transaction and VersionedTransaction without importing types.
      if (typeof tx?.sign === 'function') {
        try {
          tx.sign([keypair]);
          return tx;
        } catch {
          try {
            tx.sign(keypair);
            return tx;
          } catch {
            // fallthrough
          }
        }
      }
      if (typeof tx?.partialSign === 'function') {
        tx.partialSign(keypair);
        return tx;
      }
      throw new Error('Unsupported Solana transaction type for signing');
    },
  };
}

function normalizeHexPrivateKey(pk: string): string {
  const s = pk.trim();
  if (s.startsWith('0x')) return s;
  return `0x${s}`;
}

function expandTilde(p: string): string {
  if (!p.startsWith('~/')) return p;
  const home = (globalThis as any)?.process?.env?.HOME;
  if (!home) return p;
  return `${home}${p.slice(1)}`;
}
