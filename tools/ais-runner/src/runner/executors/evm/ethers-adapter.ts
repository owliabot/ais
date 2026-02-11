import { requireFromTsSdk } from '../../../deps.js';
import type {
  EvmExecutorOptions,
  RunnerEvmSigner,
  RunnerEvmTxRequest,
  RunnerJsonRpcTransport,
} from '../types.js';
import { asRecord, getString } from '../util.js';

interface EthersFeeData {
  maxFeePerGas?: bigint | null;
  maxPriorityFeePerGas?: bigint | null;
  gasPrice?: bigint | null;
}

interface EthersPreparedTx {
  chainId: number;
  to: string;
  data: string;
  value: bigint;
  nonce: number;
  gasLimit?: bigint;
  maxFeePerGas?: bigint;
  maxPriorityFeePerGas?: bigint;
  gasPrice?: bigint;
}

interface EthersProviderLike {
  send<T = unknown>(method: string, params: unknown[]): Promise<T>;
  estimateGas(tx: EthersPreparedTx & { from: string }): Promise<bigint>;
  getFeeData(): Promise<EthersFeeData>;
  getTransactionCount(address: string, blockTag: 'pending' | 'latest'): Promise<number>;
  destroy?: () => void;
}

interface EthersWalletLike {
  getAddress(): Promise<string>;
  signTransaction(tx: EthersPreparedTx): Promise<string>;
}

interface EthersModuleLike {
  JsonRpcProvider: new (rpcUrl: string) => EthersProviderLike;
  Wallet: new (privateKey: string, provider: EthersProviderLike) => EthersWalletLike;
}

export function createEthersTransport(rpcUrl: string): {
  provider: EthersProviderLike;
  transport: RunnerJsonRpcTransport;
} {
  const loaded = requireFromTsSdk('ethers') as unknown;
  const ethers = loaded as EthersModuleLike;
  const provider = new ethers.JsonRpcProvider(rpcUrl);
  const transport: RunnerJsonRpcTransport = {
    async request<T = unknown>(method: string, params: unknown[]): Promise<T> {
      return await provider.send<T>(method, params);
    },
  };
  return { provider, transport };
}

export function createEvmSignerFromConfig(
  signerCfg: Record<string, unknown> | undefined,
  provider: EthersProviderLike
): RunnerEvmSigner | undefined {
  if (!signerCfg) return undefined;
  const type = getString(signerCfg, 'type');
  if (type !== 'evm_private_key') return undefined;

  const envName = getString(signerCfg, 'private_key_env');
  const env = typeof process !== 'undefined' ? process.env : undefined;
  const privateKey = envName ? env?.[envName] : getString(signerCfg, 'private_key');
  if (!privateKey) return undefined;

  const loaded = requireFromTsSdk('ethers') as unknown;
  const ethers = loaded as EthersModuleLike;
  const wallet = new ethers.Wallet(normalizeHexPrivateKey(privateKey), provider);
  return new EthersBackedEvmSigner(wallet, provider);
}

class EthersBackedEvmSigner implements RunnerEvmSigner {
  private nextNonce: number | undefined;

  constructor(
    private readonly wallet: EthersWalletLike,
    private readonly provider: EthersProviderLike
  ) {}

  async signTransaction(tx: RunnerEvmTxRequest): Promise<string> {
    const from = await this.wallet.getAddress();
    for (let attempt = 0; attempt < 2; attempt++) {
      const nonce = await this.reserveNonce(from, attempt > 0);
      const base: EthersPreparedTx = {
        chainId: tx.chainId,
        to: tx.to,
        data: tx.data,
        value: tx.value,
        nonce,
      };

      try {
        base.gasLimit = await this.provider.estimateGas({ ...base, from });

        const feeData = await this.provider.getFeeData();
        if (feeData.maxFeePerGas != null && feeData.maxPriorityFeePerGas != null) {
          base.maxFeePerGas = feeData.maxFeePerGas;
          base.maxPriorityFeePerGas = feeData.maxPriorityFeePerGas;
        } else {
          base.gasPrice = await this.resolveGasPrice(feeData);
        }

        return await this.wallet.signTransaction(base);
      } catch (error) {
        if (attempt === 0 && isNonceExpiredError(error)) continue;
        throw error;
      }
    }
    throw new Error('Failed to sign transaction after nonce refresh retry');
  }

  private async resolveGasPrice(feeData: EthersFeeData): Promise<bigint> {
    if (feeData.gasPrice != null) return BigInt(feeData.gasPrice);
    const quantity = await this.provider.send<unknown>('eth_gasPrice', []);
    if (typeof quantity === 'string') return BigInt(quantity);
    throw new Error(`Unable to resolve gas price: ${String(quantity)}`);
  }

  private async reserveNonce(from: string, forceRefresh: boolean): Promise<number> {
    const pending = Number(await this.provider.getTransactionCount(from, 'pending'));
    if (!Number.isFinite(pending) || pending < 0) {
      throw new Error(`Invalid nonce from provider: ${String(pending)}`);
    }
    if (forceRefresh || this.nextNonce === undefined || this.nextNonce < pending) {
      this.nextNonce = pending;
    }
    const nonce = this.nextNonce;
    this.nextNonce = nonce + 1;
    return nonce;
  }
}

function isNonceExpiredError(error: unknown): boolean {
  const rec = asRecord(error);
  if (!rec) return false;
  const code = rec.code !== undefined ? String(rec.code) : '';
  const message = typeof rec.message === 'string' ? rec.message.toLowerCase() : '';
  return code === 'NONCE_EXPIRED' || message.includes('nonce too low') || message.includes('nonce has already been used');
}

function normalizeHexPrivateKey(privateKey: string): string {
  const trimmed = privateKey.trim();
  return trimmed.startsWith('0x') ? trimmed : `0x${trimmed}`;
}
