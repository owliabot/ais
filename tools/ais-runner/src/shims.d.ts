declare const process: {
  argv: string[];
  stdout: { write(chunk: string): void };
  env?: Record<string, string | undefined>;
  exitCode?: number;
};

declare module 'node:fs/promises' {
  export function readFile(path: string, encoding: 'utf-8' | 'utf8'): Promise<string>;
  export function writeFile(path: string, data: string, encoding: 'utf-8' | 'utf8'): Promise<void>;
  export function mkdir(path: string, options?: { recursive?: boolean }): Promise<void>;
}

declare module 'node:module' {
  export function createRequire(filename: string | URL): (id: string) => unknown;
}

declare module 'node:path' {
  export function dirname(p: string): string;
}

declare module 'yaml' {
  const YAML: { parse(src: string): unknown };
  export default YAML;
}

declare module '../../../ts-sdk/dist/index.js' {
  const mod: unknown;
  export = mod;
}

declare const fetch: (
  url: string,
  init?: unknown
) => Promise<{ json(): Promise<unknown> }>;

declare class Buffer {
  static from(data: Uint8Array): Buffer;
  toString(encoding: 'base64'): string;
}
