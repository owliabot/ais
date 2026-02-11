import type { RunnerConfig } from '../../config.js';
import { CAIP2_RE, asNonEmptyString, asPositiveInt, isRecord, type ValidationIssue } from './shared.js';
import { validateSendOptions } from './send-options.js';
import { validateSigner } from './signer.js';

type ChainConfig = NonNullable<RunnerConfig['chains']>[string];

export function validateChains(
  value: unknown,
  issues: ValidationIssue[]
): RunnerConfig['chains'] | undefined {
  if (!isRecord(value)) {
    issues.push({ path: ['chains'], message: 'chains must be an object' });
    return undefined;
  }

  const out: NonNullable<RunnerConfig['chains']> = {};
  for (const [chainId, rawChainConfig] of Object.entries(value)) {
    if (!CAIP2_RE.test(chainId)) {
      issues.push({
        path: ['chains', chainId],
        message: 'Expected CAIP-2 chain id like "eip155:1"',
      });
      continue;
    }
    if (!isRecord(rawChainConfig)) {
      issues.push({
        path: ['chains', chainId],
        message: 'chain config must be an object',
      });
      continue;
    }

    const chainConfig: ChainConfig = {};

    if (rawChainConfig.rpc_url !== undefined) {
      const rpcUrl = asNonEmptyString(rawChainConfig.rpc_url);
      if (!rpcUrl) {
        issues.push({
          path: ['chains', chainId, 'rpc_url'],
          message: 'rpc_url must be a non-empty string when provided',
        });
      } else {
        chainConfig.rpc_url = rpcUrl;
      }
    }

    if (rawChainConfig.wait_for_receipt !== undefined) {
      if (typeof rawChainConfig.wait_for_receipt !== 'boolean') {
        issues.push({
          path: ['chains', chainId, 'wait_for_receipt'],
          message: 'wait_for_receipt must be a boolean',
        });
      } else {
        chainConfig.wait_for_receipt = rawChainConfig.wait_for_receipt;
      }
    }

    if (rawChainConfig.receipt_poll !== undefined) {
      if (!isRecord(rawChainConfig.receipt_poll)) {
        issues.push({
          path: ['chains', chainId, 'receipt_poll'],
          message: 'receipt_poll must be an object',
        });
      } else {
        const receiptPoll: NonNullable<ChainConfig['receipt_poll']> = {};
        if (rawChainConfig.receipt_poll.interval_ms !== undefined) {
          const parsed = asPositiveInt(rawChainConfig.receipt_poll.interval_ms);
          if (parsed === null) {
            issues.push({
              path: ['chains', chainId, 'receipt_poll', 'interval_ms'],
              message: 'interval_ms must be a positive integer',
            });
          } else {
            receiptPoll.interval_ms = parsed;
          }
        }
        if (rawChainConfig.receipt_poll.max_attempts !== undefined) {
          const parsed = asPositiveInt(rawChainConfig.receipt_poll.max_attempts);
          if (parsed === null) {
            issues.push({
              path: ['chains', chainId, 'receipt_poll', 'max_attempts'],
              message: 'max_attempts must be a positive integer',
            });
          } else {
            receiptPoll.max_attempts = parsed;
          }
        }
        chainConfig.receipt_poll = receiptPoll;
      }
    }

    if (rawChainConfig.commitment !== undefined) {
      const commitment = asNonEmptyString(rawChainConfig.commitment);
      if (!commitment) {
        issues.push({
          path: ['chains', chainId, 'commitment'],
          message: 'commitment must be a non-empty string',
        });
      } else {
        chainConfig.commitment = commitment;
      }
    }

    if (rawChainConfig.wait_for_confirmation !== undefined) {
      if (typeof rawChainConfig.wait_for_confirmation !== 'boolean') {
        issues.push({
          path: ['chains', chainId, 'wait_for_confirmation'],
          message: 'wait_for_confirmation must be a boolean',
        });
      } else {
        chainConfig.wait_for_confirmation = rawChainConfig.wait_for_confirmation;
      }
    }

    if (rawChainConfig.send_options !== undefined) {
      const sendOptions = validateSendOptions(chainId, rawChainConfig.send_options, issues);
      if (sendOptions) chainConfig.send_options = sendOptions;
    }

    if (rawChainConfig.signer !== undefined) {
      const signer = validateSigner(chainId, rawChainConfig.signer, issues);
      if (signer) chainConfig.signer = signer;
    }

    out[chainId] = chainConfig;
  }

  return out;
}
