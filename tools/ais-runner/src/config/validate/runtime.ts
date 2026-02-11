import type { RunnerConfig } from '../../config.js';
import { isRecord, type ValidationIssue } from './shared.js';

export function validateRuntime(value: unknown, issues: ValidationIssue[]): RunnerConfig['runtime'] | undefined {
  if (!isRecord(value)) {
    issues.push({ path: ['runtime'], message: 'runtime must be an object' });
    return undefined;
  }
  const out: NonNullable<RunnerConfig['runtime']> = {};
  if (value.ctx !== undefined) {
    if (!isRecord(value.ctx)) {
      issues.push({ path: ['runtime', 'ctx'], message: 'ctx must be an object' });
    } else {
      out.ctx = value.ctx;
    }
  }
  return out;
}
