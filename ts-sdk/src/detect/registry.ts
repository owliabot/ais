import type { Detect } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import type { DetectResolver } from '../resolver/value-ref.js';

export interface DetectProvider {
  kind: string;
  provider: string;
  resolve(detect: Detect, ctx: ResolverContext): unknown | Promise<unknown>;
}

export class DetectProviderRegistry {
  private readonly byKey = new Map<string, DetectProvider>();

  register(kind: string, provider: string, resolve: DetectProvider['resolve']): void {
    if (!kind) throw new Error('DetectProvider.kind is required');
    if (!provider) throw new Error('DetectProvider.provider is required');
    const key = `${kind}:${provider}`;
    this.byKey.set(key, { kind, provider, resolve });
  }

  get(kind: string, provider: string): DetectProvider | null {
    const key = `${kind}:${provider}`;
    return this.byKey.get(key) ?? null;
  }

  list(kind?: string): DetectProvider[] {
    const all = Array.from(this.byKey.values());
    if (!kind) return all;
    return all.filter((p) => p.kind === kind);
  }

  clone(): DetectProviderRegistry {
    const r = new DetectProviderRegistry();
    for (const p of this.byKey.values()) {
      r.register(p.kind, p.provider, p.resolve);
    }
    return r;
  }
}

export interface CreateDetectResolverOptions {
  /**
   * When detect.provider is missing, allow resolving if exactly one provider
   * is registered for the kind (otherwise error).
   */
  allow_implicit_provider?: boolean;
}

export function createDetectResolver(
  registry: DetectProviderRegistry,
  options: CreateDetectResolverOptions = {}
): DetectResolver {
  const allowImplicit = options.allow_implicit_provider ?? true;

  return {
    resolve(detect, ctx) {
      const kind = detect.kind;
      const provider = detect.provider;

      if (provider) {
        const p = registry.get(kind, provider);
        if (!p) throw new Error(`Detect provider not found: kind=${kind} provider=${provider}`);
        return p.resolve(detect, ctx);
      }

      const candidates = registry.list(kind);
      if (!allowImplicit) {
        throw new Error(`Detect provider missing for kind=${kind}`);
      }
      if (candidates.length === 1) {
        return candidates[0]!.resolve(detect, ctx);
      }
      if (candidates.length === 0) {
        throw new Error(`Detect kind "${kind}" unsupported (no providers registered)`);
      }
      throw new Error(`Detect provider missing for kind=${kind} (registered: ${candidates.map((c) => c.provider).join(', ')})`);
    },
  };
}

export function createDetectProviderRegistry(): DetectProviderRegistry {
  return new DetectProviderRegistry();
}

export const defaultDetectProviderRegistry = new DetectProviderRegistry();

export function registerDetectProvider(kind: string, provider: string, resolve: DetectProvider['resolve']): void {
  defaultDetectProviderRegistry.register(kind, provider, resolve);
}

// Built-in minimal provider: choose_one/builtin (deterministic: first candidate)
registerDetectProvider('choose_one', 'builtin', (detect) => {
  const candidates = detect.candidates ?? [];
  if (!Array.isArray(candidates) || candidates.length === 0) {
    throw new Error('detect.choose_one requires non-empty candidates');
  }
  return candidates[0];
});

