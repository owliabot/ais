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
 * Param builder helper
 */
export interface ParamDef {
  name: string;
  type: string;
  required?: boolean;
  description?: string;
  default?: unknown;
}

export function param(name: string, type: string, options?: Omit<ParamDef, 'name' | 'type'>): ParamDef {
  return { name, type, ...options };
}

/**
 * Common output definition
 */
export interface OutputDef {
  name: string;
  type: string;
  path?: string;
  description?: string;
}

export function output(name: string, type: string, options?: Omit<OutputDef, 'name' | 'type'>): OutputDef {
  return { name, type, ...options };
}
