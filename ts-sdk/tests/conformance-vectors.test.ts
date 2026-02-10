import { describe, it, expect } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  canonicalizeJcs,
  specHashKeccak256,
  evaluateCEL,
  encodeJsonAbiFunctionCall,
  selectExecutionSpec,
  parseProtocolSpec,
  parseWorkflow,
  createContext,
  registerProtocol,
  buildWorkflowExecutionPlan,
} from '../src/index.js';

type VectorFile = {
  schema: 'ais-conformance/0.0.2';
  cases: VectorCase[];
};

type VectorCase =
  | {
      id: string;
      kind: 'jcs_canonicalize';
      input: { value: unknown };
      expect: { canonical: string; specHashKeccak256: string };
    }
  | {
      id: string;
      kind: 'cel_eval';
      input: { expression: string; context?: Record<string, unknown> };
      expect: { value_bigint: string };
    }
  | {
      id: string;
      kind: 'cel_eval_string';
      input: { expression: string; context?: Record<string, unknown> };
      expect: { value_string: string };
    }
  | {
      id: string;
      kind: 'cel_eval_error';
      input: { expression: string; context?: Record<string, unknown> };
      expect: { message_includes?: string };
    }
  | {
      id: string;
      kind: 'evm_json_abi_encode';
      input: { abi: any; args: Record<string, unknown> };
      expect: { data: string };
    }
  | {
      id: string;
      kind: 'select_execution_spec';
      input: { chain: string; execution: Record<string, any> };
      expect: { type: string };
    }
  | {
      id: string;
      kind: 'select_execution_spec_error';
      input: { chain: string; execution: Record<string, any> };
      expect: { message_includes?: string };
    }
  | {
      id: string;
      kind: 'workflow_plan';
      input: { protocols_yaml: string[]; workflow_yaml: string; golden_file: string };
    };

describe('AIS conformance vectors (specs/conformance/vectors)', () => {
  const root = resolve(process.cwd(), '..'); // ts-sdk/ -> repo root
  const vectorsDir = resolve(root, 'specs', 'conformance', 'vectors');
  const goldenDir = resolve(root, 'specs', 'conformance', 'golden');

  const files = readdirSync(vectorsDir).filter((f) => f.endsWith('.json')).sort();
  expect(files.length).toBeGreaterThan(0);

  for (const file of files) {
    const raw = readFileSync(resolve(vectorsDir, file), 'utf-8');
    const vf = JSON.parse(raw) as VectorFile;

    it(`${file}: schema`, () => {
      expect(vf.schema).toBe('ais-conformance/0.0.2');
      expect(Array.isArray(vf.cases)).toBe(true);
    });

    for (const c of vf.cases) {
      it(`${file} :: ${c.id}`, () => {
        switch (c.kind) {
          case 'jcs_canonicalize': {
            const got = canonicalizeJcs(c.input.value);
            expect(got).toBe(c.expect.canonical);
            expect(specHashKeccak256(c.input.value)).toBe(c.expect.specHashKeccak256);
            return;
          }
          case 'cel_eval': {
            const v = evaluateCEL(c.input.expression, c.input.context ?? {});
            expect(typeof v).toBe('bigint');
            expect(String(v as bigint)).toBe(c.expect.value_bigint);
            return;
          }
          case 'cel_eval_string': {
            const v = evaluateCEL(c.input.expression, c.input.context ?? {});
            expect(typeof v).toBe('string');
            expect(String(v)).toBe(c.expect.value_string);
            return;
          }
          case 'cel_eval_error': {
            let err: unknown;
            try {
              evaluateCEL(c.input.expression, c.input.context ?? {});
            } catch (e) {
              err = e;
            }
            expect(err).toBeTruthy();
            if (c.expect.message_includes) {
              expect(String((err as any)?.message ?? err)).toContain(c.expect.message_includes);
            }
            return;
          }
          case 'evm_json_abi_encode': {
            const data = encodeJsonAbiFunctionCall(c.input.abi, c.input.args);
            expect(data).toBe(c.expect.data);
            return;
          }
          case 'select_execution_spec': {
            const spec = selectExecutionSpec(c.input.execution as any, c.input.chain);
            expect((spec as any).type).toBe(c.expect.type);
            return;
          }
          case 'select_execution_spec_error': {
            let err: unknown;
            try {
              selectExecutionSpec(c.input.execution as any, c.input.chain);
            } catch (e) {
              err = e;
            }
            expect(err).toBeTruthy();
            if (c.expect.message_includes) {
              expect(String((err as any)?.message ?? err)).toContain(c.expect.message_includes);
            }
            return;
          }
          case 'workflow_plan': {
            const ctx = createContext();
            for (const y of c.input.protocols_yaml) {
              registerProtocol(ctx, parseProtocolSpec(y));
            }
            const wf = parseWorkflow(c.input.workflow_yaml);
            const plan = buildWorkflowExecutionPlan(wf, ctx);
            const normalized = normalizePlanForGolden(plan);

            const goldenPath = resolve(goldenDir, c.input.golden_file);
            const golden = JSON.parse(readFileSync(goldenPath, 'utf-8'));

            expect(normalized).toEqual(golden);
            return;
          }
          default: {
            const neverKind: never = c;
            throw new Error(`Unknown vector kind: ${(neverKind as any).kind}`);
          }
        }
      });
    }
  }

  function normalizePlanForGolden(plan: any): any {
    // Make timestamps deterministic for golden comparison.
    const cloned = JSON.parse(JSON.stringify(plan));
    if (cloned.meta && typeof cloned.meta === 'object') {
      if ('created_at' in cloned.meta) cloned.meta.created_at = '<ignored>';
      // omit description if undefined in source (JSON.stringify already removed undefined)
    }
    return cloned;
  }
});
