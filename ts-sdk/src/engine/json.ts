import { Buffer } from 'node:buffer';

export const AIS_JSON_TYPE_KEY = '__ais_json_type' as const;
export const AIS_JSON_CODEC_PROFILE_VERSION = 'ais-json/1' as const;

type Tagged =
  | { __ais_json_type: 'bigint'; value: string }
  | { __ais_json_type: 'uint8array'; encoding: 'base64'; value: string }
  | { __ais_json_type: 'error'; name: string; message: string; stack?: string };

export interface StringifyAisJsonOptions {
  pretty?: boolean;
  include_error_stack?: boolean;
  reject_undefined?: boolean;
  reject_non_finite_number?: boolean;
}

export interface AisJsonCodecProfile {
  version: typeof AIS_JSON_CODEC_PROFILE_VERSION;
  type_key: typeof AIS_JSON_TYPE_KEY;
  bigint: { tag: 'bigint'; format: 'decimal_string' };
  bytes: { tag: 'uint8array'; encoding: 'base64' };
  error: { tag: 'error'; fields: readonly ['name', 'message', 'stack?']; stack_default: 'strip' };
  reject: { undefined: 'strict_option'; non_finite_number: 'strict_option' };
}

export const AIS_JSON_CODEC_PROFILE: AisJsonCodecProfile = {
  version: AIS_JSON_CODEC_PROFILE_VERSION,
  type_key: AIS_JSON_TYPE_KEY,
  bigint: { tag: 'bigint', format: 'decimal_string' },
  bytes: { tag: 'uint8array', encoding: 'base64' },
  error: { tag: 'error', fields: ['name', 'message', 'stack?'], stack_default: 'strip' },
  reject: { undefined: 'strict_option', non_finite_number: 'strict_option' },
};

export interface AisJsonCodec {
  readonly profile: AisJsonCodecProfile;
  stringify(value: unknown, options?: StringifyAisJsonOptions): string;
  parse(json: string): unknown;
}

export function stringifyAisJson(value: unknown, options: StringifyAisJsonOptions = {}): string {
  return JSON.stringify(value, createAisJsonReplacer(options), options.pretty ? 2 : undefined);
}

export function parseAisJson(json: string): unknown {
  return JSON.parse(json, aisJsonReviver) as unknown;
}

export const aisJsonCodec: AisJsonCodec = {
  profile: AIS_JSON_CODEC_PROFILE,
  stringify: stringifyAisJson,
  parse: parseAisJson,
};

export function createAisJsonReplacer(options: StringifyAisJsonOptions = {}) {
  const includeErrorStack = options.include_error_stack === true;
  const rejectUndefined = options.reject_undefined ?? false;
  const rejectNonFinite = options.reject_non_finite_number ?? false;

  return function aisJsonReplacerWithOptions(_key: string, value: unknown): unknown {
    if (value === undefined) {
      if (rejectUndefined) throw new Error('AIS JSON encode failed: undefined value is not allowed');
      return value;
    }
    if (typeof value === 'number' && rejectNonFinite && !Number.isFinite(value)) {
      throw new Error('AIS JSON encode failed: non-finite number is not allowed');
    }
    if (typeof value === 'function' || typeof value === 'symbol') {
      throw new Error('AIS JSON encode failed: function/symbol values are not allowed');
    }
    if (typeof value === 'bigint') {
      const tagged: Tagged = { [AIS_JSON_TYPE_KEY]: 'bigint', value: value.toString() } as const;
      return tagged;
    }
    if (value instanceof Uint8Array) {
      const tagged: Tagged = {
        [AIS_JSON_TYPE_KEY]: 'uint8array',
        encoding: 'base64',
        value: Buffer.from(value).toString('base64'),
      } as const;
      return tagged;
    }
    if (value instanceof Error) {
      const tagged: Tagged = {
        [AIS_JSON_TYPE_KEY]: 'error',
        name: value.name,
        message: value.message,
        ...(includeErrorStack && typeof value.stack === 'string' ? { stack: value.stack } : {}),
      };
      return tagged;
    }
    return value;
  };
}

export function aisJsonReplacer(_key: string, value: unknown): unknown {
  return createAisJsonReplacer()(_key, value);
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
    (v.stack === undefined || typeof v.stack === 'string') &&
    (Object.keys(v).length === 3 || Object.keys(v).length === 4)
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
