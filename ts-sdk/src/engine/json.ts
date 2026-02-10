import { Buffer } from 'node:buffer';

export const AIS_JSON_TYPE_KEY = '__ais_json_type' as const;

type Tagged =
  | { __ais_json_type: 'bigint'; value: string }
  | { __ais_json_type: 'uint8array'; encoding: 'base64'; value: string }
  | { __ais_json_type: 'error'; name: string; message: string; stack?: string };

export interface StringifyAisJsonOptions {
  pretty?: boolean;
}

export function stringifyAisJson(value: unknown, options: StringifyAisJsonOptions = {}): string {
  return JSON.stringify(value, aisJsonReplacer, options.pretty ? 2 : undefined);
}

export function parseAisJson(json: string): unknown {
  return JSON.parse(json, aisJsonReviver) as unknown;
}

export function aisJsonReplacer(_key: string, value: unknown): unknown {
  if (typeof value === 'bigint') {
    const tagged: Tagged = { [AIS_JSON_TYPE_KEY]: 'bigint', value: value.toString() } as any;
    return tagged;
  }
  if (value instanceof Uint8Array) {
    const tagged: Tagged = {
      [AIS_JSON_TYPE_KEY]: 'uint8array',
      encoding: 'base64',
      value: Buffer.from(value).toString('base64'),
    } as any;
    return tagged;
  }
  if (value instanceof Error) {
    const tagged: Tagged = {
      [AIS_JSON_TYPE_KEY]: 'error',
      name: value.name,
      message: value.message,
      stack: value.stack,
    };
    return tagged;
  }
  return value;
}

export function aisJsonReviver(_key: string, value: unknown): unknown {
  if (!value || typeof value !== 'object') return value;
  const v = value as Record<string, unknown>;

  if (v[AIS_JSON_TYPE_KEY] === 'bigint' && typeof v.value === 'string' && Object.keys(v).length === 2) {
    return BigInt(v.value);
  }

  if (
    v[AIS_JSON_TYPE_KEY] === 'uint8array' &&
    v.encoding === 'base64' &&
    typeof v.value === 'string' &&
    Object.keys(v).length === 3
  ) {
    return new Uint8Array(Buffer.from(v.value, 'base64'));
  }

  if (
    v[AIS_JSON_TYPE_KEY] === 'error' &&
    typeof v.name === 'string' &&
    typeof v.message === 'string' &&
    (v.stack === undefined || typeof v.stack === 'string')
  ) {
    const err = new Error(v.message);
    err.name = v.name;
    if (typeof v.stack === 'string') {
      (err as { stack?: string }).stack = v.stack;
    }
    return err;
  }

  return value;
}

