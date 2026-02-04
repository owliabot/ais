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
  type AISDocumentType,
} from './schema/index.js';

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
}

/**
 * Generic parser factory - creates type-safe parsers from Zod schemas
 */
function createParser<T>(schema: ZodSchema<T>, typeName: string) {
  return (yaml: string, options: ParseOptions = {}): T => {
    const { source } = options;

    let parsed: unknown;
    try {
      parsed = parseYAML(yaml);
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

    return result.data;
  };
}

/** Parse any AIS document (auto-detects type) */
export const parseAIS = createParser<AnyAISDocument>(AISDocumentSchema, 'Document');

/** Parse a Protocol Spec */
export const parseProtocolSpec = createParser<ProtocolSpec>(ProtocolSpecSchema, 'Protocol spec');

/** Parse a Pack */
export const parsePack = createParser<Pack>(PackSchema, 'Pack');

/** Parse a Workflow */
export const parseWorkflow = createParser<Workflow>(WorkflowSchema, 'Workflow');

/**
 * Detect document type without full validation
 */
export function detectType(yaml: string): AISDocumentType | null {
  try {
    const parsed = parseYAML(yaml);
    if (typeof parsed === 'object' && parsed !== null && 'type' in parsed) {
      const type = (parsed as Record<string, unknown>).type;
      if (type === 'protocol' || type === 'pack' || type === 'workflow') {
        return type;
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
    const parsed = parseYAML(yaml);
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
