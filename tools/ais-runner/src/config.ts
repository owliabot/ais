import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { validateRunnerConfigOrThrow } from './config-validate.js';

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
  const doc = yaml.parse(expanded) as unknown;
  if (!doc || typeof doc !== 'object') throw new Error('Invalid config: expected mapping object');
  return validateRunnerConfigOrThrow(doc) as RunnerConfig;
}

export function expandEnv(input: string): string {
  return input.replace(/\$\{([A-Z0-9_]+)\}/g, (_m, name) => {
    const v = (globalThis as any)?.process?.env?.[name];
    return v === undefined ? '' : String(v);
  });
}

async function loadYamlParser(): Promise<{ parse(src: string): any }> {
  // Prefer local dependency (tools/ais-runner/node_modules) when installed.
  try {
    const m = (await import('yaml')) as any;
    return m?.default ?? m;
  } catch {
    // Fallback: resolve from ts-sdk's dependency tree (useful in repo dev without installing runner deps).
    const tsSdkPkg = new URL('../../../ts-sdk/package.json', import.meta.url);
    const req = createRequire(tsSdkPkg);
    return req('yaml');
  }
}
