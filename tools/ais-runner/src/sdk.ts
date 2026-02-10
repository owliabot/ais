let cached: any | null = null;

export async function loadSdk(): Promise<any> {
  if (cached) return cached;
  cached = await import('../../../ts-sdk/dist/index.js');
  return cached;
}

