import type { RunnerConfig } from '../../config.js';
import { asPositiveInt, CAIP2_RE, isRecord, type ValidationIssue } from './shared.js';

export function validateEngine(value: unknown, issues: ValidationIssue[]): RunnerConfig['engine'] | undefined {
  if (!isRecord(value)) {
    issues.push({ path: ['engine'], message: 'engine must be an object' });
    return undefined;
  }

  const out: NonNullable<RunnerConfig['engine']> = {};

  if (value.max_concurrency !== undefined) {
    const parsed = asPositiveInt(value.max_concurrency);
    if (parsed === null) {
      issues.push({
        path: ['engine', 'max_concurrency'],
        message: 'max_concurrency must be a positive integer',
      });
    } else {
      out.max_concurrency = parsed;
    }
  }

  if (value.per_chain !== undefined) {
    if (!isRecord(value.per_chain)) {
      issues.push({ path: ['engine', 'per_chain'], message: 'per_chain must be an object' });
    } else {
      const perChain: NonNullable<NonNullable<RunnerConfig['engine']>['per_chain']> = {};
      for (const [chainId, rawRule] of Object.entries(value.per_chain)) {
        if (!CAIP2_RE.test(chainId)) {
          issues.push({
            path: ['engine', 'per_chain', chainId],
            message: 'Expected CAIP-2 chain id like "eip155:1"',
          });
          continue;
        }
        if (!isRecord(rawRule)) {
          issues.push({
            path: ['engine', 'per_chain', chainId],
            message: 'per-chain rule must be an object',
          });
          continue;
        }
        const rule: { max_read_concurrency?: number; max_write_concurrency?: number } = {};
        if (rawRule.max_read_concurrency !== undefined) {
          const parsed = asPositiveInt(rawRule.max_read_concurrency);
          if (parsed === null) {
            issues.push({
              path: ['engine', 'per_chain', chainId, 'max_read_concurrency'],
              message: 'max_read_concurrency must be a positive integer',
            });
          } else {
            rule.max_read_concurrency = parsed;
          }
        }
        if (rawRule.max_write_concurrency !== undefined) {
          const parsed = asPositiveInt(rawRule.max_write_concurrency);
          if (parsed === null) {
            issues.push({
              path: ['engine', 'per_chain', chainId, 'max_write_concurrency'],
              message: 'max_write_concurrency must be a positive integer',
            });
          } else {
            rule.max_write_concurrency = parsed;
          }
        }
        perChain[chainId] = rule;
      }
      out.per_chain = perChain;
    }
  }

  return out;
}
