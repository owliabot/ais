import { z } from 'zod';
import { RuntimePatchSchema, type RuntimePatch } from './patch.js';

export const RunnerCommandKindSchema = z.enum([
  'apply_patches',
  'user_confirm',
  'select_provider',
  'cancel',
]);

export const ApplyPatchesPayloadSchema = z
  .object({
    patches: z.array(RuntimePatchSchema).min(1),
  })
  .strict();

export const UserConfirmPayloadSchema = z
  .object({
    node_id: z.string().min(1),
    approve: z.boolean().default(true),
  })
  .strict();

export const SelectProviderPayloadSchema = z
  .object({
    node_id: z.string().min(1).optional(),
    detect_kind: z.string().min(1),
    provider: z.string().min(1),
    chain: z.string().min(1).optional(),
  })
  .strict();

export const CancelPayloadSchema = z
  .object({
    node_id: z.string().min(1).optional(),
    reason: z.string().min(1).optional(),
  })
  .strict();

export const RunnerCommandSchema = z
  .object({
    id: z.string().min(1),
    ts: z.string().min(1),
    kind: RunnerCommandKindSchema,
    payload: z.unknown(),
    extensions: z.record(z.unknown()).optional(),
  })
  .superRefine((value, ctx) => {
    const result = validateCommandPayload(value.kind, value.payload);
    if (result.ok) return;
    for (const issue of result.error.issues) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: issue.message,
        path: issue.path.length > 0 ? ['payload', ...issue.path] : ['payload'],
      });
    }
  });

export type RunnerCommandKind = z.infer<typeof RunnerCommandKindSchema>;
export type ApplyPatchesPayload = z.infer<typeof ApplyPatchesPayloadSchema>;
export type UserConfirmPayload = z.infer<typeof UserConfirmPayloadSchema>;
export type SelectProviderPayload = z.infer<typeof SelectProviderPayloadSchema>;
export type CancelPayload = z.infer<typeof CancelPayloadSchema>;
export type RunnerCommand = z.infer<typeof RunnerCommandSchema>;

export type ParsedRunnerCommand =
  | (RunnerCommand & { kind: 'apply_patches'; payload: ApplyPatchesPayload })
  | (RunnerCommand & { kind: 'user_confirm'; payload: UserConfirmPayload })
  | (RunnerCommand & { kind: 'select_provider'; payload: SelectProviderPayload })
  | (RunnerCommand & { kind: 'cancel'; payload: CancelPayload });

export type CommandValidationError = {
  reason: string;
  field_path?: string;
  details?: unknown;
};

export function validateRunnerCommand(input: unknown): { ok: true; command: ParsedRunnerCommand } | { ok: false; error: CommandValidationError } {
  const parsed = RunnerCommandSchema.safeParse(input);
  if (!parsed.success) {
    const first = parsed.error.issues[0];
    const fieldPath = first?.path?.join('.') || undefined;
    return {
      ok: false,
      error: {
        reason: first?.message ?? 'invalid command',
        field_path: fieldPath,
        details: parsed.error.issues.map((issue) => ({
          path: issue.path.join('.'),
          message: issue.message,
          code: issue.code,
        })),
      },
    };
  }

  const command = parsed.data;
  const payload = parseTypedPayload(command.kind, command.payload);
  if (!payload.ok) {
    return {
      ok: false,
      error: payload.error,
    };
  }

  return {
    ok: true,
    command: {
      ...command,
      payload: payload.payload,
    } as ParsedRunnerCommand,
  };
}

function parseTypedPayload(
  kind: RunnerCommandKind,
  payload: unknown
): { ok: true; payload: ApplyPatchesPayload | UserConfirmPayload | SelectProviderPayload | CancelPayload } | { ok: false; error: CommandValidationError } {
  const parser =
    kind === 'apply_patches'
      ? ApplyPatchesPayloadSchema
      : kind === 'user_confirm'
        ? UserConfirmPayloadSchema
        : kind === 'select_provider'
          ? SelectProviderPayloadSchema
          : CancelPayloadSchema;

  const parsed = parser.safeParse(payload);
  if (!parsed.success) {
    const first = parsed.error.issues[0];
    return {
      ok: false,
      error: {
        reason: first?.message ?? 'invalid command payload',
        field_path: first?.path?.length ? `payload.${first.path.join('.')}` : 'payload',
        details: parsed.error.issues.map((issue) => ({
          path: issue.path.join('.'),
          message: issue.message,
          code: issue.code,
        })),
      },
    };
  }
  return { ok: true, payload: parsed.data };
}

function validateCommandPayload(kind: RunnerCommandKind, payload: unknown): { ok: true } | { ok: false; error: z.ZodError } {
  if (kind === 'apply_patches') {
    const parsed = ApplyPatchesPayloadSchema.safeParse(payload);
    if (!parsed.success) return { ok: false, error: parsed.error };
    return { ok: true };
  }
  if (kind === 'user_confirm') {
    const parsed = UserConfirmPayloadSchema.safeParse(payload);
    if (!parsed.success) return { ok: false, error: parsed.error };
    return { ok: true };
  }
  if (kind === 'select_provider') {
    const parsed = SelectProviderPayloadSchema.safeParse(payload);
    if (!parsed.success) return { ok: false, error: parsed.error };
    return { ok: true };
  }
  const parsed = CancelPayloadSchema.safeParse(payload);
  if (!parsed.success) return { ok: false, error: parsed.error };
  return { ok: true };
}

export function summarizeCommand(command: ParsedRunnerCommand): { id: string; ts: string; kind: RunnerCommandKind; patch_count?: number } {
  if (command.kind === 'apply_patches') {
    return {
      id: command.id,
      ts: command.ts,
      kind: command.kind,
      patch_count: command.payload.patches.length,
    };
  }
  return {
    id: command.id,
    ts: command.ts,
    kind: command.kind,
  };
}

export function commandPatches(command: ParsedRunnerCommand): RuntimePatch[] {
  return command.kind === 'apply_patches' ? (command.payload.patches as RuntimePatch[]) : [];
}
