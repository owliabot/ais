import { writeFile, mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';

export type EvaluatedOutputs = {
  outputs: Record<string, unknown>;
  errors: Record<string, string>;
};

export function evaluateWorkflowOutputs(sdk: any, workflow: any, ctx: any): EvaluatedOutputs {
  const out: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  const outputs = workflow?.outputs;
  if (!outputs || typeof outputs !== 'object') return { outputs: out, errors };

  for (const [k, v] of Object.entries(outputs)) {
    try {
      out[k] = sdk.evaluateValueRef(v, ctx);
    } catch (e) {
      errors[k] = (e as Error)?.message ?? String(e);
    }
  }

  return { outputs: out, errors };
}

export async function writeOutputsJson(filePath: string, data: unknown): Promise<void> {
  await mkdir(dirname(filePath), { recursive: true });
  const raw = stringifyWithBigInt(data);
  await writeFile(filePath, `${raw}\n`, 'utf-8');
}

export function stringifyWithBigInt(value: unknown): string {
  return JSON.stringify(
    value,
    (_k, v) => (typeof v === 'bigint' ? v.toString() : v),
    2
  );
}

