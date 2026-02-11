import { createRequire } from 'node:module';

const tsSdkPkg = new URL('../../../ts-sdk/package.json', import.meta.url);
const req = createRequire(tsSdkPkg);

export function requireFromTsSdk<T = unknown>(id: string): T {
  return req(id) as T;
}
