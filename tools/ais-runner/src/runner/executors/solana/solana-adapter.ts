import { readFile } from 'node:fs/promises';
import { requireFromTsSdk } from '../../../deps.js';
import type {
  RunnerChainConfig,
  RunnerSolanaConnection,
  RunnerSolanaSigner,
  SolanaExecutorOptions,
  SolanaSignTxInput,
} from '../types.js';
import { expandTilde, getString, isNumberArray } from '../util.js';

interface SolanaKeypairLike {
  publicKey: RunnerSolanaSigner['publicKey'];
}

interface SolanaWeb3ModuleLike {
  Connection: new (rpcUrl: string, commitment?: string) => RunnerSolanaConnection;
  Keypair: {
    fromSecretKey(secret: Uint8Array): SolanaKeypairLike;
  };
}

interface SolanaSignableTransaction {
  sign?: (signers: unknown) => unknown;
  partialSign?: (signer: unknown) => unknown;
}

export function createSolanaConnection(rpcUrl: string, commitment: string | undefined): RunnerSolanaConnection {
  const loaded = requireFromTsSdk('@solana/web3.js') as unknown;
  const web3 = loaded as SolanaWeb3ModuleLike;
  return new web3.Connection(rpcUrl, commitment ?? 'confirmed');
}

export async function createSolanaSignerFromConfig(
  signerCfg: Record<string, unknown> | undefined
): Promise<RunnerSolanaSigner | undefined> {
  if (!signerCfg) return undefined;
  const type = getString(signerCfg, 'type');
  if (type !== 'solana_keypair_file') return undefined;
  const keypairPath = getString(signerCfg, 'keypair_path');
  if (!keypairPath) return undefined;

  const expandedPath = expandTilde(keypairPath);
  const raw = await readFile(expandedPath, 'utf-8');
  const parsed = JSON.parse(raw) as unknown;
  if (!isNumberArray(parsed)) {
    throw new Error('Invalid Solana keypair file: expected JSON number array');
  }

  const loaded = requireFromTsSdk('@solana/web3.js') as unknown;
  const web3 = loaded as SolanaWeb3ModuleLike;
  const secret = Uint8Array.from(parsed);
  const keypair = web3.Keypair.fromSecretKey(secret);

  const signer: RunnerSolanaSigner = {
    publicKey: keypair.publicKey,
    signTransaction(tx: SolanaSignTxInput): SolanaSignTxInput {
      const signable = tx as unknown as SolanaSignableTransaction;
      if (typeof signable.sign === 'function') {
        try {
          signable.sign([keypair]);
          return tx;
        } catch {
          try {
            signable.sign(keypair);
            return tx;
          } catch {
            // fall through to partialSign
          }
        }
      }
      if (typeof signable.partialSign === 'function') {
        signable.partialSign(keypair);
        return tx;
      }
      throw new Error('Unsupported Solana transaction type for signing');
    },
  };
  return signer;
}

export function toCommitment(value: string | undefined): SolanaExecutorOptions['commitment'] {
  if (!value) return undefined;
  if (
    value === 'processed' ||
    value === 'confirmed' ||
    value === 'finalized' ||
    value === 'recent' ||
    value === 'single' ||
    value === 'singleGossip' ||
    value === 'root' ||
    value === 'max'
  ) {
    return value;
  }
  return undefined;
}

export function toSendOptions(
  options: RunnerChainConfig['send_options'] | undefined
): SolanaExecutorOptions['send_options'] {
  if (!options) return undefined;
  return {
    skipPreflight: options.skipPreflight,
    maxRetries: options.maxRetries,
    preflightCommitment: toCommitment(options.preflightCommitment),
  };
}
