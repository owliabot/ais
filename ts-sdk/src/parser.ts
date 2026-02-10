/**
 * Parser module - parse and validate AIS documents from YAML
 */
import { parse as parseYAML } from 'yaml';
import type { ZodSchema } from 'zod';
import {
  AISDocumentSchema,
  ProtocolSpecSchema,
  PackSchema,
  WorkflowSchema,
  type AnyAISDocument,
  type ProtocolSpec,
  type Pack,
  type Workflow,
  type AISSchemaType,
} from './schema/index.js';
import type { ExecutionTypeRegistry } from './plugins/index.js';
import { defaultExecutionTypeRegistry, validateProtocolExecutionTypes } from './plugins/index.js';
import { assertProtocolSemantics } from './validator/index.js';

export class AISParseError extends Error {
  constructor(
    message: string,
    public readonly source?: string,
    public readonly details?: unknown
  ) {
    super(message);
    this.name = 'AISParseError';
  }
}

export interface ParseOptions {
  /** Source identifier for error messages */
  source?: string;
  /**
   * Execution type registry used to validate non-core execution specs.
   * When omitted, `defaultExecutionTypeRegistry` is used (register via `registerExecutionType()`).
   */
  execution_registry?: ExecutionTypeRegistry;
}

/**
 * Generic parser factory - creates type-safe parsers from Zod schemas
 */
function createParser<T>(
  schema: ZodSchema<T>,
  typeName: string,
  postValidate?: (doc: T, options: ParseOptions) => void
) {
  return (yaml: string, options: ParseOptions = {}): T => {
    const { source } = options;

    let parsed: unknown;
    try {
      parsed = parseYAML(yaml, { uniqueKeys: true });
    } catch (err) {
      throw new AISParseError(
        `Invalid YAML: ${err instanceof Error ? err.message : 'Unknown error'}`,
        source
      );
    }

    const result = schema.safeParse(parsed);
    if (!result.success) {
      const issues = result.error.issues
        .map((i) => `${i.path.join('.')}: ${i.message}`)
        .join('; ');
      throw new AISParseError(
        `${typeName} validation failed: ${issues}`,
        source,
        result.error.issues
      );
    }

    if (postValidate) {
      try {
        postValidate(result.data, options);
      } catch (err) {
        throw new AISParseError(err instanceof Error ? err.message : String(err), source, (err as any)?.details);
      }
    }

    return result.data;
  };
}

/** Parse any AIS document (auto-detects type) */
export const parseAIS = createParser<AnyAISDocument>(AISDocumentSchema, 'Document', (doc, options) => {
  if (doc.schema !== 'ais/0.0.2') return;
  const registry = options.execution_registry ?? defaultExecutionTypeRegistry;
  validateProtocolExecutionTypes(doc, { registry, source: options.source });
  assertProtocolSemantics(doc);
});

/** Parse a Protocol Spec */
export const parseProtocolSpec = createParser<ProtocolSpec>(ProtocolSpecSchema, 'Protocol spec', (spec, options) => {
  const registry = options.execution_registry ?? defaultExecutionTypeRegistry;
  validateProtocolExecutionTypes(spec, { registry, source: options.source });
  assertProtocolSemantics(spec);
});

/** Parse a Pack */
export const parsePack = createParser<Pack>(PackSchema, 'Pack');

/** Parse a Workflow */
export const parseWorkflow = createParser<Workflow>(WorkflowSchema, 'Workflow');

/**
 * Detect document type without full validation
 */
export function detectType(yaml: string): AISSchemaType | null {
  try {
    const parsed = parseYAML(yaml, { uniqueKeys: true });
    if (typeof parsed === 'object' && parsed !== null && 'schema' in parsed) {
      const schema = (parsed as Record<string, unknown>).schema;
      if (
        schema === 'ais/0.0.2' ||
        schema === 'ais-pack/0.0.2' ||
        schema === 'ais-flow/0.0.2'
      ) {
        return schema;
      }
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Validate document without parsing (returns issues or null)
 */
export function validate(
  yaml: string
): { valid: true } | { valid: false; issues: string[] } {
  try {
    const parsed = parseYAML(yaml, { uniqueKeys: true });
    const result = AISDocumentSchema.safeParse(parsed);
    if (result.success) {
      return { valid: true };
    }
    return {
      valid: false,
      issues: result.error.issues.map((i) => `${i.path.join('.')}: ${i.message}`),
    };
  } catch (err) {
    return {
      valid: false,
      issues: [`YAML parse error: ${err instanceof Error ? err.message : 'Unknown'}`],
    };
  }
}
