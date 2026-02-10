import { createRequire } from 'node:module';

export function validateRunnerConfigOrThrow(doc: unknown): any {
  const z = loadZod();

  const IntPos = z.number().int().positive();
  const ChainId = z.string().min(1).regex(/^[a-z0-9]+:.+$/i, 'Expected CAIP-2 chain id like "eip155:1"');

  const ReceiptPoll = z
    .object({
      interval_ms: IntPos.optional(),
      max_attempts: IntPos.optional(),
    })
    .strict()
    .optional();

  const Signer = z
    .object({
      type: z.string().min(1).optional(),
      private_key_env: z.string().min(1).optional(),
      private_key: z.string().min(1).optional(),
      keypair_path: z.string().min(1).optional(),
      fee_payer: z.string().min(1).optional(),
    })
    .passthrough()
    .optional()
    .superRefine((v: any, ctx: any) => {
      if (!v) return;
      const type = v.type ? String(v.type) : '';
      if (!type) return;
      if (type !== 'evm_private_key' && type !== 'solana_keypair_file') {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: `Unsupported signer.type "${type}" (supported: evm_private_key, solana_keypair_file)`,
        });
      }
      if (type === 'evm_private_key') {
        if (!v.private_key && !v.private_key_env) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: 'evm_private_key signer requires private_key or private_key_env',
          });
        }
      }
      if (type === 'solana_keypair_file') {
        if (!v.keypair_path) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: 'solana_keypair_file signer requires keypair_path',
          });
        }
      }
    });

  const ChainConfig = z
    .object({
      rpc_url: z.string().min(1).optional(),
      wait_for_receipt: z.boolean().optional(),
      receipt_poll: ReceiptPoll,
      commitment: z.string().min(1).optional(),
      wait_for_confirmation: z.boolean().optional(),
      send_options: z
        .object({
          skipPreflight: z.boolean().optional(),
          maxRetries: IntPos.optional(),
          preflightCommitment: z.string().min(1).optional(),
        })
        .strict()
        .optional(),
      signer: Signer,
    })
    .passthrough();

  const schema = z
    .object({
      schema: z.string().min(1).optional(),
      engine: z
        .object({
          max_concurrency: IntPos.optional(),
          per_chain: z
            .record(
              ChainId,
              z
                .object({
                  max_read_concurrency: IntPos.optional(),
                  max_write_concurrency: IntPos.optional(),
                })
                .strict()
            )
            .optional(),
        })
        .strict()
        .optional(),
      chains: z.record(ChainId, ChainConfig).optional(),
      runtime: z
        .object({
          ctx: z.record(z.unknown()).optional(),
        })
        .strict()
        .optional(),
    })
    .passthrough()
    .superRefine((cfg: any, ctx: any) => {
      if (!cfg.chains) return;
      for (const [chain, c] of Object.entries(cfg.chains)) {
        if (!c || typeof c !== 'object') continue;
        const rpc = (c as any).rpc_url;
        if (rpc !== undefined && (typeof rpc !== 'string' || rpc.trim().length === 0)) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ['chains', chain, 'rpc_url'],
            message: 'rpc_url must be a non-empty string when provided',
          });
        }
      }
    });

  const parsed = schema.safeParse(doc);
  if (parsed.success) return parsed.data;

  const lines = parsed.error.issues.map((i: any) => `- ${formatZodPath(i.path)}: ${i.message}`);
  throw new Error(`Invalid runner config:\n${lines.join('\n')}`);
}

function formatZodPath(path: Array<string | number>): string {
  if (!path || path.length === 0) return '(root)';
  return path
    .map((p) => (typeof p === 'number' ? `[${p}]` : /^[A-Za-z_][A-Za-z0-9_]*$/.test(p) ? p : JSON.stringify(p)))
    .join('.');
}

function loadZod(): any {
  // Prefer local dependency when installed in tools/ais-runner.
  try {
    const reqLocal = createRequire(import.meta.url);
    return reqLocal('zod');
  } catch {
    const tsSdkPkg = new URL('../../../ts-sdk/package.json', import.meta.url);
    const req = createRequire(tsSdkPkg);
    return req('zod');
  }
}

