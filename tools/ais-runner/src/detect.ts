import type {
  RunnerDetectResolver,
  RunnerSdkModule,
  RunnerWorkflow,
  RunnerWorkspaceDocuments,
} from './types.js';
import { findRequiredPackDocument } from './runner/workspace/resolve.js';

type DetectInput = Parameters<RunnerDetectResolver['resolve']>[0];
type DetectContext = Parameters<RunnerDetectResolver['resolve']>[1];
type ValueRefEvalErrorCtor = RunnerSdkModule['ValueRefEvalError'];

type ProviderEntry = {
  kind: string;
  provider: string;
  priority: number;
  resolve: (detect: DetectInput, ctx: DetectContext) => unknown | Promise<unknown>;
};

type DetectSdk = {
  ValueRefEvalError: ValueRefEvalErrorCtor;
  pickDetectProvider?: RunnerSdkModule['pickDetectProvider'];
  checkDetectAllowed?: RunnerSdkModule['checkDetectAllowed'];
};

export function createRunnerDetectResolver(args: {
  sdk: DetectSdk;
  workflow?: RunnerWorkflow;
  workspaceDocs?: RunnerWorkspaceDocuments;
}): RunnerDetectResolver | undefined {
  const { sdk, workflow, workspaceDocs } = args;
  if (!sdk) return undefined;

  const byKind = new Map<string, ProviderEntry[]>();
  const byKey = new Map<string, ProviderEntry>();

  const register = (entry: ProviderEntry) => {
    const key = `${entry.kind}:${entry.provider}`;
    byKey.set(key, entry);
    const current = byKind.get(entry.kind) ?? [];
    current.push(entry);
    byKind.set(entry.kind, current);
  };

  register({
    kind: 'best_quote',
    provider: 'builtin-first',
    priority: -100,
    resolve: (detect) => firstCandidateOrThrow(sdk.ValueRefEvalError, detect),
  });
  register({
    kind: 'best_path',
    provider: 'builtin-first',
    priority: -100,
    resolve: (detect) => firstCandidateOrThrow(sdk.ValueRefEvalError, detect),
  });
  register({
    kind: 'protocol_specific',
    provider: 'builtin-first',
    priority: -100,
    resolve: (detect) => firstCandidateOrThrow(sdk.ValueRefEvalError, detect),
  });

  const pack = workflow && workspaceDocs ? findRequiredPackDocument(workflow, workspaceDocs) : null;
  const enabledProviders = pack?.document?.providers?.detect?.enabled;
  if (Array.isArray(enabledProviders)) {
    for (const providerConfig of enabledProviders) {
      const kind = typeof providerConfig?.kind === 'string' ? providerConfig.kind : '';
      const provider = typeof providerConfig?.provider === 'string' ? providerConfig.provider : '';
      if (!kind || !provider) continue;
      const priority = typeof providerConfig?.priority === 'number' ? providerConfig.priority : 0;
      const providerCandidates = Array.isArray(providerConfig?.candidates)
        ? providerConfig.candidates
        : undefined;
      register({
        kind,
        provider,
        priority,
        resolve: (detect) => {
          const candidates =
            Array.isArray(detect.candidates) && detect.candidates.length > 0
              ? detect.candidates
              : providerCandidates;
          if (!Array.isArray(candidates) || candidates.length === 0) {
            throw new sdk.ValueRefEvalError(
              `Detect kind "${detect.kind || kind}" requires non-empty candidates`
            );
          }
          return candidates[0];
        },
      });
    }
  }

  return {
    resolve(detect: DetectInput, ctx: DetectContext): unknown | Promise<unknown> {
      const kind = detect.kind ? String(detect.kind) : '';
      const requestedProvider = detect.provider ? String(detect.provider) : null;
      if (!kind) throw new sdk.ValueRefEvalError('Detect.kind is required');
      const chain = detectChain(ctx);
      const overrideProvider = pickRuntimeProviderOverride(ctx, kind, chain);

      let provider = requestedProvider;
      if (!provider && overrideProvider) provider = overrideProvider;
      if (pack?.document && sdk.pickDetectProvider) {
        const picked = sdk.pickDetectProvider(pack.document, {
          kind,
          provider: provider ?? undefined,
          chain,
        });
        if (!picked.ok) {
          throw new sdk.ValueRefEvalError(picked.reason ?? 'detect provider blocked by pack', {
            cause: picked.details,
          });
        }
        provider = picked.provider ?? requestedProvider ?? null;
      } else if (pack?.document && sdk.checkDetectAllowed && requestedProvider) {
        const check = sdk.checkDetectAllowed(pack.document, {
          kind,
          provider: requestedProvider,
          chain,
        });
        if (!check.ok) {
          throw new sdk.ValueRefEvalError(check.reason ?? 'detect provider blocked by pack', {
            cause: check.details,
          });
        }
      }

      if (provider) {
        const picked = byKey.get(`${kind}:${provider}`);
        if (!picked) {
          throw new sdk.ValueRefEvalError(`Detect provider not found: kind=${kind} provider=${provider}`);
        }
        return picked.resolve(detect, ctx);
      }

      const candidates = byKind.get(kind) ?? [];
      if (candidates.length === 0) {
        throw new sdk.ValueRefEvalError(
          `Detect kind "${kind}" unsupported (no providers registered)`
        );
      }

      const best = candidates
        .slice()
        .sort(
          (left, right) =>
            right.priority - left.priority || left.provider.localeCompare(right.provider)
        )[0];
      if (!best) {
        throw new sdk.ValueRefEvalError(
          `Detect kind "${kind}" unsupported (no providers registered)`
        );
      }
      return best.resolve(detect, ctx);
    },
  };
}

function pickRuntimeProviderOverride(
  ctx: DetectContext,
  kind: string,
  chain: string | undefined
): string | null {
  const runtimeCtx = (ctx.runtime?.ctx ?? {}) as Record<string, unknown>;
  const list = runtimeCtx.runner_detect_overrides;
  if (!Array.isArray(list) || list.length === 0) return null;

  let best: { provider: string; score: number } | null = null;
  for (const entry of list) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) continue;
    const rec = entry as Record<string, unknown>;
    const entryKind = typeof rec.kind === 'string' ? rec.kind : '';
    const provider = typeof rec.provider === 'string' ? rec.provider : '';
    if (!entryKind || !provider) continue;
    if (entryKind !== kind) continue;

    const entryChain = typeof rec.chain === 'string' ? rec.chain : undefined;
    if (entryChain && chain && entryChain !== chain) continue;
    if (entryChain && !chain) continue;

    const score = entryChain ? 2 : 1;
    if (!best || score >= best.score) best = { provider, score };
  }
  return best?.provider ?? null;
}

function firstCandidateOrThrow(
  ValueRefEvalError: ValueRefEvalErrorCtor,
  detect: DetectInput
): unknown {
  const candidates = detect.candidates;
  if (!Array.isArray(candidates) || candidates.length === 0) {
    throw new ValueRefEvalError('Detect requires non-empty candidates');
  }
  return candidates[0];
}

function detectChain(ctx: DetectContext): string | undefined {
  const runtimeCtx = (ctx.runtime?.ctx ?? {}) as Record<string, unknown>;
  const chainId = runtimeCtx.chain_id;
  if (typeof chainId === 'string' && chainId.length > 0) return chainId;
  const chain = runtimeCtx.chain;
  if (typeof chain === 'string' && chain.length > 0) return chain;
  return undefined;
}
