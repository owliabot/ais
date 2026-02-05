/**
 * Base builder utilities and types
 */

import type { ZodSchema } from 'zod';
import { stringify } from 'yaml';

/**
 * Base builder with common functionality
 */
export abstract class BaseBuilder<T> {
  protected abstract schema: ZodSchema<T>;
  protected abstract getData(): T;

  /**
   * Build and validate the document
   */
  build(): T {
    const data = this.getData();
    return this.schema.parse(data);
  }

  /**
   * Build without validation (for partial documents)
   */
  buildUnsafe(): T {
    return this.getData();
  }

  /**
   * Convert to YAML string
   */
  toYAML(): string {
    return stringify(this.build());
  }

  /**
   * Convert to JSON string
   */
  toJSON(pretty = true): string {
    return JSON.stringify(this.build(), null, pretty ? 2 : 0);
  }
}

/**
 * Parameter definition
 */
export interface ParamDef {
  name: string;
  type: string;
  description?: string;
  required?: boolean;
  default?: unknown;
  example?: unknown;
  constraints?: {
    min?: number;
    max?: number;
    enum?: unknown[];
    pattern?: string;
  };
  /** For token_amount type - reference to asset param */
  asset_ref?: string;
}

export function param(
  name: string,
  type: string,
  options?: Omit<ParamDef, 'name' | 'type'>
): ParamDef {
  return { name, type, ...options };
}

/**
 * Output/Return field definition
 */
export interface OutputDef {
  name: string;
  type: string;
  description?: string;
}

export function output(
  name: string,
  type: string,
  options?: Omit<OutputDef, 'name' | 'type'>
): OutputDef {
  return { name, type, ...options };
}
