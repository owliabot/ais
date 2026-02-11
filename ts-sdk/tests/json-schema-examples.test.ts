import { describe, it, expect } from 'vitest';
import { readFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as parseYAML } from 'yaml';
import { WorkflowSchema } from '../src/schema/workflow.js';

type JsonSchema = Record<string, any>;
type ValidationError = { path: string; message: string };

function resolveRef(ref: string, root: JsonSchema): JsonSchema {
  if (!ref.startsWith('#/$defs/')) {
    throw new Error(`Unsupported $ref format: ${ref}`);
  }
  const name = ref.slice('#/$defs/'.length);
  const def = root.$defs?.[name];
  if (!def) throw new Error(`Missing $defs entry: ${name}`);
  return def as JsonSchema;
}

function validateSchema(schema: JsonSchema, data: any, root: JsonSchema, path: string): ValidationError[] {
  if (schema.$ref) {
    return validateSchema(resolveRef(schema.$ref as string, root), data, root, path);
  }

  if (schema.anyOf) {
    const variants = schema.anyOf as JsonSchema[];
    for (const v of variants) {
      const errs = validateSchema(v, data, root, path);
      if (errs.length === 0) return [];
    }
    return [{ path, message: 'Value does not match anyOf variants' }];
  }

  if (schema.const !== undefined) {
    if (data !== schema.const) return [{ path, message: `Expected const ${JSON.stringify(schema.const)}` }];
    return [];
  }

  if (schema.enum) {
    const list = schema.enum as any[];
    if (!list.includes(data)) return [{ path, message: `Expected enum one of ${JSON.stringify(list)}` }];
    return [];
  }

  const type = schema.type as string | undefined;
  if (!type) return [];

  switch (type) {
    case 'object': {
      if (typeof data !== 'object' || data === null || Array.isArray(data)) {
        return [{ path, message: 'Expected object' }];
      }
      const obj = data as Record<string, any>;
      const props = (schema.properties ?? {}) as Record<string, JsonSchema>;
      const required = (schema.required ?? []) as string[];
      const errs: ValidationError[] = [];

      for (const k of required) {
        if (!(k in obj)) errs.push({ path: path ? `${path}.${k}` : k, message: 'Missing required property' });
      }

      for (const [k, v] of Object.entries(props)) {
        if (k in obj) errs.push(...validateSchema(v, obj[k], root, path ? `${path}.${k}` : k));
      }

      const additional = schema.additionalProperties;
      if (additional === false) {
        for (const k of Object.keys(obj)) {
          if (!(k in props)) errs.push({ path: path ? `${path}.${k}` : k, message: 'Unknown property' });
        }
      } else if (additional && typeof additional === 'object') {
        for (const k of Object.keys(obj)) {
          if (!(k in props)) errs.push(...validateSchema(additional as JsonSchema, obj[k], root, path ? `${path}.${k}` : k));
        }
      }

      return errs;
    }

    case 'array': {
      if (!Array.isArray(data)) return [{ path, message: 'Expected array' }];
      const errs: ValidationError[] = [];
      if (schema.minItems !== undefined && data.length < schema.minItems) {
        errs.push({ path, message: `Expected minItems ${schema.minItems}` });
      }
      if (schema.maxItems !== undefined && data.length > schema.maxItems) {
        errs.push({ path, message: `Expected maxItems ${schema.maxItems}` });
      }
      const items = schema.items as JsonSchema | undefined;
      if (items) {
        for (let i = 0; i < data.length; i++) {
          errs.push(...validateSchema(items, data[i], root, `${path}[${i}]`));
        }
      }
      return errs;
    }

    case 'string': {
      if (typeof data !== 'string') return [{ path, message: 'Expected string' }];
      const errs: ValidationError[] = [];
      if (schema.minLength !== undefined && data.length < schema.minLength) {
        errs.push({ path, message: `Expected minLength ${schema.minLength}` });
      }
      if (schema.maxLength !== undefined && data.length > schema.maxLength) {
        errs.push({ path, message: `Expected maxLength ${schema.maxLength}` });
      }
      if (schema.pattern) {
        const re = new RegExp(schema.pattern as string);
        if (!re.test(data)) errs.push({ path, message: `Pattern mismatch: ${schema.pattern}` });
      }
      return errs;
    }

    case 'number':
    case 'integer': {
      if (typeof data !== 'number' || !Number.isFinite(data)) return [{ path, message: `Expected ${type}` }];
      const errs: ValidationError[] = [];
      if (type === 'integer' && !Number.isInteger(data)) errs.push({ path, message: 'Expected integer' });
      if (schema.minimum !== undefined && data < schema.minimum) errs.push({ path, message: `Expected >= ${schema.minimum}` });
      if (schema.maximum !== undefined && data > schema.maximum) errs.push({ path, message: `Expected <= ${schema.maximum}` });
      if (schema.exclusiveMinimum !== undefined && data <= schema.exclusiveMinimum) {
        errs.push({ path, message: `Expected > ${schema.exclusiveMinimum}` });
      }
      if (schema.exclusiveMaximum !== undefined && data >= schema.exclusiveMaximum) {
        errs.push({ path, message: `Expected < ${schema.exclusiveMaximum}` });
      }
      return errs;
    }

    case 'boolean':
      if (typeof data !== 'boolean') return [{ path, message: 'Expected boolean' }];
      return [];

    default:
      return [];
  }
}

function validate(rootSchema: JsonSchema, data: any): ValidationError[] {
  return validateSchema(rootSchema, data, rootSchema, '');
}

describe('published JSON Schemas', () => {
  it('validate examples against schemas/0.0.2', async () => {
    const here = dirname(fileURLToPath(import.meta.url));
    const tsSdkRoot = resolve(here, '..');
    const repoRoot = resolve(tsSdkRoot, '..');

    const schemaDir = resolve(repoRoot, 'schemas', '0.0.2');
    const examplesDir = resolve(repoRoot, 'examples');

    const protocolSchema = JSON.parse(await readFile(resolve(schemaDir, 'protocol.schema.json'), 'utf-8')) as JsonSchema;
    const packSchema = JSON.parse(await readFile(resolve(schemaDir, 'pack.schema.json'), 'utf-8')) as JsonSchema;
    const workflowSchema = JSON.parse(await readFile(resolve(schemaDir, 'workflow.schema.json'), 'utf-8')) as JsonSchema;

    const files = [
      'aave-v3.ais.yaml',
      'erc20.ais.yaml',
      'spl-token.ais.yaml',
      'bridge-demo.ais.yaml',
      'solana-rpc.ais.yaml',
      'solana-vault-demo.ais.yaml',
      'uniswap-v3.ais.yaml',
      'safe-defi-pack.ais-pack.yaml',
      'bridge-send-wait-deposit.ais-flow.yaml',
      'aave-branch-bridge-solana-deposit.ais-flow.yaml',
      'swap-to-token.ais-flow.yaml',
    ];

    for (const f of files) {
      const raw = await readFile(resolve(examplesDir, f), 'utf-8');
      const doc = parseYAML(raw) as any;
      expect(typeof doc).toBe('object');
      expect(doc && typeof doc.schema).toBe('string');

      const schema =
        doc.schema === 'ais/0.0.2'
          ? protocolSchema
          : doc.schema === 'ais-pack/0.0.2'
            ? packSchema
            : doc.schema === 'ais-flow/0.0.2'
              ? workflowSchema
              : null;

      if (doc.schema === 'ais-flow/0.0.3') {
        const parsed = WorkflowSchema.safeParse(doc);
        expect(parsed.success, `${f} workflow parse issues`).toBe(true);
      } else {
        if (!schema) throw new Error(`Unknown example schema: ${doc.schema} (${f})`);
        const errors = validate(schema, doc);
        expect(errors, `${f} errors: ${errors.map((e) => `${e.path}: ${e.message}`).join('; ')}`).toHaveLength(0);
      }
    }
  });
});
