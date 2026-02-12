import type { EngineCheckpoint } from './types.js';
import { aisJsonReplacer, aisJsonReviver, parseAisJson, stringifyAisJson } from './json.js';
import type { TraceRedactMode } from './trace.js';
import { redactSensitiveFields } from './trace.js';

export interface SerializeCheckpointOptions {
  pretty?: boolean;
  /**
   * Redaction mode applied to the serialized checkpoint payload.
   *
   * - default: redacts strict + sensitive fields (recommended for agent logs)
   * - audit: redacts secrets but keeps more structure
   * - off: no redaction (unsafe)
   */
  redact_mode?: TraceRedactMode;
  /**
   * Optional regex patterns (matched against full field_path) that disable redaction for those paths.
   *
   * This should only be used when you explicitly need additional fields for audits.
   */
  redact_allow_path_patterns?: string[];
}

export function serializeCheckpoint(
  checkpoint: EngineCheckpoint,
  options: SerializeCheckpointOptions = {}
): string {
  const mode: TraceRedactMode = options.redact_mode ?? 'default';
  const payload =
    mode === 'off'
      ? checkpoint
      : (redactSensitiveFields(checkpoint, {
          strict: mode === 'default',
          allow_path_patterns: options.redact_allow_path_patterns,
        }) as EngineCheckpoint);
  return stringifyAisJson(payload, { pretty: options.pretty });
}

export function deserializeCheckpoint(json: string): EngineCheckpoint {
  return parseAisJson(json) as EngineCheckpoint;
}

export function checkpointJsonReplacer(_key: string, value: unknown): unknown {
  return aisJsonReplacer(_key, value);
}

export function checkpointJsonReviver(_key: string, value: unknown): unknown {
  return aisJsonReviver(_key, value);
}
