import type { RunnerConfig } from '../../config.js';
import { asNonEmptyString, asPositiveInt, isRecord, type ValidationIssue } from './shared.js';

type ChainConfig = NonNullable<RunnerConfig['chains']>[string];
type SendOptionsConfig = NonNullable<ChainConfig['send_options']>;

export function validateSendOptions(
  chainId: string,
  value: unknown,
  issues: ValidationIssue[]
): SendOptionsConfig | undefined {
  if (!isRecord(value)) {
    issues.push({
      path: ['chains', chainId, 'send_options'],
      message: 'send_options must be an object',
    });
    return undefined;
  }

  const out: SendOptionsConfig = {};
  if (value.skipPreflight !== undefined) {
    if (typeof value.skipPreflight !== 'boolean') {
      issues.push({
        path: ['chains', chainId, 'send_options', 'skipPreflight'],
        message: 'skipPreflight must be a boolean',
      });
    } else {
      out.skipPreflight = value.skipPreflight;
    }
  }
  if (value.maxRetries !== undefined) {
    const parsed = asPositiveInt(value.maxRetries);
    if (parsed === null) {
      issues.push({
        path: ['chains', chainId, 'send_options', 'maxRetries'],
        message: 'maxRetries must be a positive integer',
      });
    } else {
      out.maxRetries = parsed;
    }
  }
  if (value.preflightCommitment !== undefined) {
    const parsed = asNonEmptyString(value.preflightCommitment);
    if (!parsed) {
      issues.push({
        path: ['chains', chainId, 'send_options', 'preflightCommitment'],
        message: 'preflightCommitment must be a non-empty string',
      });
    } else {
      out.preflightCommitment = parsed;
    }
  }
  return out;
}
