import { z } from 'zod';
import { ExtensionsSchema } from './common.js';

// Minimal conformance vector file schema (AIS 0.0.2)

const JcsCanonicalizeCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('jcs_canonicalize'),
    input: z.object({ value: z.unknown(), extensions: ExtensionsSchema }).strict(),
    expect: z
      .object({
        canonical: z.string(),
        specHashKeccak256: z.string(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const CelEvalCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('cel_eval'),
    input: z
      .object({
        expression: z.string(),
        context: z.record(z.unknown()).optional(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        value_bigint: z.string(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const CelEvalStringCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('cel_eval_string'),
    input: z
      .object({
        expression: z.string(),
        context: z.record(z.unknown()).optional(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        value_string: z.string(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const CelEvalErrorCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('cel_eval_error'),
    input: z
      .object({
        expression: z.string(),
        context: z.record(z.unknown()).optional(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        message_includes: z.string().optional(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const EvmJsonAbiEncodeCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('evm_json_abi_encode'),
    input: z
      .object({
        abi: z.unknown(),
        args: z.record(z.unknown()),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        data: z.string(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const SelectExecutionSpecCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('select_execution_spec'),
    input: z
      .object({
        chain: z.string(),
        execution: z.record(z.unknown()),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        type: z.string(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const SelectExecutionSpecErrorCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('select_execution_spec_error'),
    input: z
      .object({
        chain: z.string(),
        execution: z.record(z.unknown()),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        message_includes: z.string().optional(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    extensions: ExtensionsSchema,
  })
  .strict();

const WorkflowPlanCaseSchema = z
  .object({
    id: z.string(),
    kind: z.literal('workflow_plan'),
    input: z
      .object({
        protocols_yaml: z.array(z.string()),
        workflow_yaml: z.string(),
        golden_file: z.string(),
        extensions: ExtensionsSchema,
      })
      .strict(),
    expect: z
      .object({
        extensions: ExtensionsSchema,
      })
      .strict()
      .optional(),
    extensions: ExtensionsSchema,
  })
  .strict();

export const ConformanceCaseSchema = z.union([
  JcsCanonicalizeCaseSchema,
  CelEvalCaseSchema,
  CelEvalStringCaseSchema,
  CelEvalErrorCaseSchema,
  EvmJsonAbiEncodeCaseSchema,
  SelectExecutionSpecCaseSchema,
  SelectExecutionSpecErrorCaseSchema,
  WorkflowPlanCaseSchema,
]);

export const ConformanceVectorFileSchema = z
  .object({
    schema: z.literal('ais-conformance/0.0.2'),
    cases: z.array(ConformanceCaseSchema),
    extensions: ExtensionsSchema,
  })
  .strict();

export type ConformanceVectorFile = z.infer<typeof ConformanceVectorFileSchema>;
export type ConformanceCase = z.infer<typeof ConformanceCaseSchema>;
