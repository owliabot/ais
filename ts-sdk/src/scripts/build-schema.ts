import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { z } from 'zod';

import {
  ProtocolSpecSchema,
  PackSchema,
  WorkflowSchema,
  ConformanceVectorFileSchema,
} from '../schema/index.js';
import { ExecutionPlanSchema } from '../execution/plan.js';

type JsonSchema = Record<string, unknown>;

type ZodTypeAny = z.ZodTypeAny;

type ConvertContext = {
  defs: Record<string, JsonSchema>;
  seen: Map<ZodTypeAny, string>;
  nextId: number;
};

function createCtx(): ConvertContext {
  return { defs: {}, seen: new Map(), nextId: 1 };
}

function ref(defName: string): JsonSchema {
  return { $ref: `#/$defs/${defName}` };
}

function sanitizeDefName(name: string): string {
  const s = name.replace(/[^a-zA-Z0-9_]/g, '_').replace(/_+/g, '_');
  return s.length > 60 ? s.slice(0, 60) : s;
}

function convert(schema: ZodTypeAny, ctx: ConvertContext, hint?: string): JsonSchema {
  const typeName = (schema as any)?._def?.typeName as z.ZodFirstPartyTypeKind | undefined;

  // Handle recursion via $defs (important for ZodLazy graphs).
  const already = ctx.seen.get(schema);
  if (already) return ref(already);

  if (typeName === z.ZodFirstPartyTypeKind.ZodLazy) {
    const base = sanitizeDefName(hint ?? 'Lazy');
    const name = `${base}_${ctx.nextId++}`;
    ctx.seen.set(schema, name);
    const getter = (schema as z.ZodLazy<any>)._def.getter as () => ZodTypeAny;
    const out = convertInline(getter(), ctx);
    ctx.defs[name] = out;
    return ref(name);
  }

  // Unwrap wrappers that don't affect JSON shape.
  if (typeName === z.ZodFirstPartyTypeKind.ZodOptional) {
    return convert((schema as z.ZodOptional<any>)._def.innerType, ctx, hint);
  }
  if (typeName === z.ZodFirstPartyTypeKind.ZodDefault) {
    return convert((schema as z.ZodDefault<any>)._def.innerType, ctx, hint);
  }
  if (typeName === z.ZodFirstPartyTypeKind.ZodEffects) {
    return convert((schema as z.ZodEffects<any>)._def.schema, ctx, hint);
  }

  const base = sanitizeDefName(hint ?? 'Def');
  const name = `${base}_${ctx.nextId++}`;
  const shouldDef =
    typeName === z.ZodFirstPartyTypeKind.ZodObject ||
    typeName === z.ZodFirstPartyTypeKind.ZodUnion ||
    typeName === z.ZodFirstPartyTypeKind.ZodDiscriminatedUnion ||
    typeName === z.ZodFirstPartyTypeKind.ZodRecord;

  if (shouldDef) {
    ctx.seen.set(schema, name);
    const out = convertInline(schema, ctx);
    ctx.defs[name] = out;
    return ref(name);
  }

  return convertInline(schema, ctx);
}

function convertInline(schema: ZodTypeAny, ctx: ConvertContext): JsonSchema {
  const def = (schema as any)?._def;
  const typeName = def?.typeName as z.ZodFirstPartyTypeKind | undefined;

  switch (typeName) {
    case z.ZodFirstPartyTypeKind.ZodUnknown:
    case z.ZodFirstPartyTypeKind.ZodAny:
      return {};

    case z.ZodFirstPartyTypeKind.ZodBoolean:
      return { type: 'boolean' };

    case z.ZodFirstPartyTypeKind.ZodLiteral:
      return { const: def.value };

    case z.ZodFirstPartyTypeKind.ZodEnum:
      return { type: 'string', enum: def.values.slice() };

    case z.ZodFirstPartyTypeKind.ZodString: {
      const s: JsonSchema = { type: 'string' };
      for (const check of def.checks ?? []) {
        if (check.kind === 'min') s.minLength = check.value;
        if (check.kind === 'max') s.maxLength = check.value;
        if (check.kind === 'regex') s.pattern = String(check.regex.source);
        if (check.kind === 'url') s.format = 'uri';
      }
      return s;
    }

    case z.ZodFirstPartyTypeKind.ZodNumber: {
      const n: JsonSchema = { type: 'number' };
      let isInt = false;
      for (const check of def.checks ?? []) {
        if (check.kind === 'int') isInt = true;
      }
      if (isInt) n.type = 'integer';
      for (const check of def.checks ?? []) {
        if (check.kind === 'min') {
          if (check.inclusive) n.minimum = check.value;
          else n.exclusiveMinimum = check.value;
        }
        if (check.kind === 'max') {
          if (check.inclusive) n.maximum = check.value;
          else n.exclusiveMaximum = check.value;
        }
        if (check.kind === 'positive') {
          if (n.type === 'integer') n.minimum = Math.max(1, Number(n.minimum ?? 1));
          else n.exclusiveMinimum = 0;
        }
        if (check.kind === 'nonnegative') {
          n.minimum = 0;
        }
      }
      return n;
    }

    case z.ZodFirstPartyTypeKind.ZodArray: {
      const items = convert(def.type, ctx);
      const out: JsonSchema = { type: 'array', items };
      if (def.minLength) out.minItems = def.minLength.value;
      if (def.maxLength) out.maxItems = def.maxLength.value;
      return out;
    }

    case z.ZodFirstPartyTypeKind.ZodRecord: {
      const valueSchema = convert(def.valueType, ctx);
      const out: JsonSchema = { type: 'object', additionalProperties: valueSchema };
      return out;
    }

    case z.ZodFirstPartyTypeKind.ZodUnion: {
      const options = def.options as ZodTypeAny[];
      return { anyOf: options.map((opt, i) => convert(opt, ctx, `Union${ctx.nextId}_${i}`)) };
    }

    case z.ZodFirstPartyTypeKind.ZodDiscriminatedUnion: {
      const options = Array.from(def.options.values()) as ZodTypeAny[];
      return { anyOf: options.map((opt, i) => convert(opt, ctx, `DU${ctx.nextId}_${i}`)) };
    }

    case z.ZodFirstPartyTypeKind.ZodObject: {
      const shape = def.shape();
      const props: Record<string, JsonSchema> = {};
      const required: string[] = [];
      for (const [k, v] of Object.entries(shape)) {
        const inner = v as ZodTypeAny;
        const isOptional =
          (inner as any)?._def?.typeName === z.ZodFirstPartyTypeKind.ZodOptional ||
          (inner as any)?._def?.typeName === z.ZodFirstPartyTypeKind.ZodDefault;
        props[k] = convert(inner, ctx, `${k}`);
        if (!isOptional) required.push(k);
      }

      const out: JsonSchema = { type: 'object', properties: props };
      if (required.length > 0) out.required = required;

      // unknownKeys: 'strict' | 'strip' | 'passthrough'
      const unknownKeys = def.unknownKeys as string | undefined;
      if (unknownKeys === 'passthrough') {
        out.additionalProperties = true;
      } else {
        // strict (reject) and strip are both treated as reject in published schemas.
        out.additionalProperties = false;
      }
      return out;
    }

    default:
      return {};
  }
}

function buildRootSchema(title: string, id: string, root: ZodTypeAny): JsonSchema {
  const ctx = createCtx();
  const schemaRef = convert(root, ctx, 'Root');
  const out: JsonSchema = {
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    $id: id,
    title,
    ...schemaRef,
  };
  if (Object.keys(ctx.defs).length > 0) {
    out.$defs = ctx.defs;
  }
  return out;
}

async function writeJson(filePath: string, data: unknown): Promise<void> {
  await mkdir(dirname(filePath), { recursive: true });
  await writeFile(filePath, `${JSON.stringify(data, null, 2)}\n`, 'utf-8');
}

async function main(): Promise<void> {
  const here = dirname(fileURLToPath(import.meta.url));
  const tsSdkRoot = resolve(here, '..', '..');
  const repoRoot = resolve(tsSdkRoot, '..');

  const outDir = resolve(repoRoot, 'schemas', '0.0.2');

  const protocol = buildRootSchema('AIS Protocol Spec (ais/0.0.2)', 'urn:ais:0.0.2:protocol', ProtocolSpecSchema);
  const pack = buildRootSchema('AIS Pack (ais-pack/0.0.2)', 'urn:ais:0.0.2:pack', PackSchema);
  const workflow = buildRootSchema('AIS Workflow (ais-flow/0.0.3)', 'urn:ais:0.0.3:workflow', WorkflowSchema);
  const plan = buildRootSchema('AIS Execution Plan (ais-plan/0.0.3)', 'urn:ais:0.0.2:plan', ExecutionPlanSchema);
  const conformance = buildRootSchema(
    'AIS Conformance Vectors (ais-conformance/0.0.2)',
    'urn:ais:0.0.2:conformance',
    ConformanceVectorFileSchema
  );

  await mkdir(outDir, { recursive: true });
  await writeJson(resolve(outDir, 'protocol.schema.json'), protocol);
  await writeJson(resolve(outDir, 'pack.schema.json'), pack);
  await writeJson(resolve(outDir, 'workflow.schema.json'), workflow);
  await writeJson(resolve(outDir, 'plan.schema.json'), plan);
  await writeJson(resolve(outDir, 'conformance.schema.json'), conformance);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
