import type { Writable } from 'node:stream';
import { createInterface } from 'node:readline';
import type { Readable } from 'node:stream';
import { parseAisJson, stringifyAisJson } from '../json.js';

export interface JsonlRpcPeerOptions {
  input: Readable;
  output: Writable;
  /**
   * When true, invalid JSON lines are ignored instead of throwing.
   */
  ignore_invalid_json?: boolean;
}

export interface JsonlRpcPeer {
  send(message: unknown): void;
  messages(): AsyncGenerator<unknown>;
  close(): void;
}

/**
 * Minimal JSONL peer for process-to-process integration.
 *
 * This is intentionally generic and does not prescribe a specific "AIS RPC" method set.
 * Upstream systems can define their own message envelopes (request/response/notify).
 *
 * Notes:
 * - Uses the AIS JSON codec so BigInt / Uint8Array / Error can be transported safely.
 * - One JSON object per line.
 */
export function createJsonlRpcPeer(options: JsonlRpcPeerOptions): JsonlRpcPeer {
  const ignoreInvalid = options.ignore_invalid_json ?? false;
  const rl = createInterface({ input: options.input, crlfDelay: Infinity });

  return {
    send(message) {
      options.output.write(`${stringifyAisJson(message)}\n`);
    },
    async *messages() {
      for await (const line of rl) {
        const trimmed = String(line).trim();
        if (!trimmed) continue;
        try {
          yield parseAisJson(trimmed);
        } catch (e) {
          if (ignoreInvalid) continue;
          throw e;
        }
      }
    },
    close() {
      rl.close();
    },
  };
}

