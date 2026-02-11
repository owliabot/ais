import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { validateRunnerConfigOrThrow } from './config/validate/index.js';

export type RunnerConfig = {
  schema?: string;
  engine?: {
    max_concurrency?: number;
    per_chain?: Record<
      string,
      {
        max_read_concurrency?: number;
        max_write_concurrency?: number;
      }
    >;
  };
  chains?: Record<
    string,
    {
      rpc_url?: string;
      wait_for_receipt?: boolean;
      receipt_poll?: { interval_ms?: number; max_attempts?: number };
      commitment?: string;
      wait_for_confirmation?: boolean;
      send_options?: {
        skipPreflight?: boolean;
        maxRetries?: number;
        preflightCommitment?: string;
      };
      signer?: {
        type?: string;
        private_key_env?: string;
        private_key?: string;
        keypair_path?: string;
        fee_payer?: string;
      } & Record<string, unknown>;
    }
  >;
  runtime?: {
    ctx?: Record<string, unknown>;
  };
};

export async function loadRunnerConfig(filePath: string): Promise<RunnerConfig> {
  const raw = await readFile(filePath, 'utf-8');
  const expanded = expandEnv(raw);
  const yaml = await loadYamlParser();
  const doc = yaml.parse(expanded);
  if (!doc || typeof doc !== 'object') throw new Error('Invalid config: expected mapping object');
  return validateRunnerConfigOrThrow(doc);
}

export function expandEnv(input: string): string {
  return input.replace(/\$\{([A-Z0-9_]+)\}/g, (_m, name) => {
    const v = process.env?.[name];
    return v === undefined ? '' : String(v);
  });
}

type YamlParser = { parse(src: string): unknown };

async function loadYamlParser(): Promise<YamlParser> {
  // Prefer local dependency (tools/ais-runner/node_modules) when installed.
  try {
    const loaded = await import('yaml');
    const parser = isYamlParser(loaded.default) ? loaded.default : isYamlParser(loaded) ? loaded : null;
    if (parser) return parser;
    throw new Error('Invalid yaml module: missing parse()');
  } catch {
    // Fallback: resolve from ts-sdk's dependency tree (useful in repo dev without installing runner deps).
    const tsSdkPkg = new URL('../../../ts-sdk/package.json', import.meta.url);
    const req = createRequire(tsSdkPkg);
    const parser = req('yaml');
    if (!isYamlParser(parser)) throw new Error('Invalid yaml module from ts-sdk: missing parse()');
    return parser;
  }
}

function isYamlParser(value: unknown): value is YamlParser {
  if (!value || typeof value !== 'object') return false;
  const rec = value as Record<string, unknown>;
  return typeof rec.parse === 'function';
}
