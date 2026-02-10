import type { EngineCheckpoint } from './types.js';
import { aisJsonReplacer, aisJsonReviver, parseAisJson, stringifyAisJson } from './json.js';

export interface SerializeCheckpointOptions {
  pretty?: boolean;
}

export function serializeCheckpoint(
  checkpoint: EngineCheckpoint,
  options: SerializeCheckpointOptions = {}
): string {
  return stringifyAisJson(checkpoint, { pretty: options.pretty });
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
