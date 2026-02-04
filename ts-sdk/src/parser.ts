/**
 * AIS Protocol SDK - Parser
 * Load and validate AIS documents from YAML
 */

import { parse as parseYAML } from 'yaml';
import { AISDocumentSchema, ProtocolSpecSchema, PackSchema, WorkflowSchema } from './schema.js';
import type { AnyAISDocument, ProtocolSpec, Pack, Workflow, AISDocumentType } from './types.js';

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
  /** Strict mode fails on unknown fields */
  strict?: boolean;
  /** Source identifier for error messages */
  source?: string;
}

/**
 * Parse a YAML string into an AIS document
 */
export function parseAIS(yaml: string, options: ParseOptions = {}): AnyAISDocument {
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

  if (typeof parsed !== 'object' || parsed === null) {
    throw new AISParseError('Document must be an object', source);
  }

  const result = AISDocumentSchema.safeParse(parsed);
  if (!result.success) {
    throw new AISParseError(
      `Validation failed: ${result.error.issues.map(i => `${i.path.join('.')}: ${i.message}`).join('; ')}`,
      source,
      result.error.issues
    );
  }

  return result.data;
}

/**
 * Parse a YAML string as a Protocol Spec
 */
export function parseProtocolSpec(yaml: string, options: ParseOptions = {}): ProtocolSpec {
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

  const result = ProtocolSpecSchema.safeParse(parsed);
  if (!result.success) {
    throw new AISParseError(
      `Protocol spec validation failed: ${result.error.issues.map(i => `${i.path.join('.')}: ${i.message}`).join('; ')}`,
      source,
      result.error.issues
    );
  }

  return result.data;
}

/**
 * Parse a YAML string as a Pack
 */
export function parsePack(yaml: string, options: ParseOptions = {}): Pack {
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

  const result = PackSchema.safeParse(parsed);
  if (!result.success) {
    throw new AISParseError(
      `Pack validation failed: ${result.error.issues.map(i => `${i.path.join('.')}: ${i.message}`).join('; ')}`,
      source,
      result.error.issues
    );
  }

  return result.data;
}

/**
 * Parse a YAML string as a Workflow
 */
export function parseWorkflow(yaml: string, options: ParseOptions = {}): Workflow {
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

  const result = WorkflowSchema.safeParse(parsed);
  if (!result.success) {
    throw new AISParseError(
      `Workflow validation failed: ${result.error.issues.map(i => `${i.path.join('.')}: ${i.message}`).join('; ')}`,
      source,
      result.error.issues
    );
  }

  return result.data;
}

/**
 * Detect the document type from YAML without full validation
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
 * Validate a document without parsing (returns issues or null)
 */
export function validate(yaml: string): { valid: true } | { valid: false; issues: string[] } {
  try {
    const parsed = parseYAML(yaml);
    const result = AISDocumentSchema.safeParse(parsed);
    if (result.success) {
      return { valid: true };
    }
    return {
      valid: false,
      issues: result.error.issues.map(i => `${i.path.join('.')}: ${i.message}`),
    };
  } catch (err) {
    return {
      valid: false,
      issues: [`YAML parse error: ${err instanceof Error ? err.message : 'Unknown'}`],
    };
  }
}
