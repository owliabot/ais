import type { RunnerConfig } from '../../config.js';
import { validateEngine } from './engine.js';
import { validateChains } from './chains.js';
import { validateRuntime } from './runtime.js';
import { asNonEmptyString, formatPath, isRecord, type ValidationIssue } from './shared.js';

export function validateRunnerConfigOrThrow(doc: unknown): RunnerConfig {
  const issues: ValidationIssue[] = [];
  const out: RunnerConfig = {};

  if (!isRecord(doc)) {
    throw new Error('Invalid runner config:\n- (root): expected object');
  }

  if (doc.schema !== undefined) {
    const schema = asNonEmptyString(doc.schema);
    if (!schema) {
      issues.push({
        path: ['schema'],
        message: 'schema must be a non-empty string when provided',
      });
    } else {
      out.schema = schema;
    }
  }

  if (doc.engine !== undefined) {
    const engine = validateEngine(doc.engine, issues);
    if (engine) out.engine = engine;
  }

  if (doc.chains !== undefined) {
    const chains = validateChains(doc.chains, issues);
    if (chains) out.chains = chains;
  }

  if (doc.runtime !== undefined) {
    const runtime = validateRuntime(doc.runtime, issues);
    if (runtime) out.runtime = runtime;
  }

  if (issues.length > 0) {
    const lines = issues.map((issue) => `- ${formatPath(issue.path)}: ${issue.message}`);
    throw new Error(`Invalid runner config:\n${lines.join('\n')}`);
  }

  return out;
}
