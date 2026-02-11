import type { RunnerConfig } from '../../config.js';
import { asNonEmptyString, isRecord, type ValidationIssue } from './shared.js';

type ChainConfig = NonNullable<RunnerConfig['chains']>[string];
type SignerConfig = NonNullable<ChainConfig['signer']>;

export function validateSigner(
  chainId: string,
  value: unknown,
  issues: ValidationIssue[]
): SignerConfig | undefined {
  if (!isRecord(value)) {
    issues.push({
      path: ['chains', chainId, 'signer'],
      message: 'signer must be an object',
    });
    return undefined;
  }

  const out: SignerConfig = { ...value };
  const type = value.type !== undefined ? asNonEmptyString(value.type) : undefined;
  if (value.type !== undefined && !type) {
    issues.push({
      path: ['chains', chainId, 'signer', 'type'],
      message: 'type must be a non-empty string',
    });
  }
  if (type) out.type = type;

  const privateKeyEnv = value.private_key_env !== undefined ? asNonEmptyString(value.private_key_env) : undefined;
  if (value.private_key_env !== undefined && !privateKeyEnv) {
    issues.push({
      path: ['chains', chainId, 'signer', 'private_key_env'],
      message: 'private_key_env must be a non-empty string',
    });
  }
  if (privateKeyEnv) out.private_key_env = privateKeyEnv;

  const privateKey = value.private_key !== undefined ? asNonEmptyString(value.private_key) : undefined;
  if (value.private_key !== undefined && !privateKey) {
    issues.push({
      path: ['chains', chainId, 'signer', 'private_key'],
      message: 'private_key must be a non-empty string',
    });
  }
  if (privateKey) out.private_key = privateKey;

  const keypairPath = value.keypair_path !== undefined ? asNonEmptyString(value.keypair_path) : undefined;
  if (value.keypair_path !== undefined && !keypairPath) {
    issues.push({
      path: ['chains', chainId, 'signer', 'keypair_path'],
      message: 'keypair_path must be a non-empty string',
    });
  }
  if (keypairPath) out.keypair_path = keypairPath;

  const feePayer = value.fee_payer !== undefined ? asNonEmptyString(value.fee_payer) : undefined;
  if (value.fee_payer !== undefined && !feePayer) {
    issues.push({
      path: ['chains', chainId, 'signer', 'fee_payer'],
      message: 'fee_payer must be a non-empty string',
    });
  }
  if (feePayer) out.fee_payer = feePayer;

  if (type && type !== 'evm_private_key' && type !== 'solana_keypair_file') {
    issues.push({
      path: ['chains', chainId, 'signer', 'type'],
      message:
        `Unsupported signer.type "${type}" (supported: evm_private_key, solana_keypair_file)`,
    });
  }
  if (type === 'evm_private_key' && !privateKey && !privateKeyEnv) {
    issues.push({
      path: ['chains', chainId, 'signer'],
      message: 'evm_private_key signer requires private_key or private_key_env',
    });
  }
  if (type === 'solana_keypair_file' && !keypairPath) {
    issues.push({
      path: ['chains', chainId, 'signer'],
      message: 'solana_keypair_file signer requires keypair_path',
    });
  }

  return out;
}
