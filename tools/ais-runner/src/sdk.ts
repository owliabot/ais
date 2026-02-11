import type { RunnerSdkModule } from './types.js';

let cached: RunnerSdkModule | null = null;

export async function loadSdk(): Promise<RunnerSdkModule> {
  if (cached) return cached;
  cached = (await import('../../../ts-sdk/dist/index.js')) as RunnerSdkModule;
  return cached;
}
