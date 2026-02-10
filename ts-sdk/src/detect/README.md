# Detect Module

Pluggable providers for `{ detect: ... }` ValueRefs (AIS `0.0.2`).

`detect` is used for dynamic selection/resolution (e.g. choose among candidates, fetch quotes, compute routes).

## File Structure

| File | Purpose |
|------|---------|
| `registry.ts` | `DetectProviderRegistry` + default registry + `createDetectResolver()` |
| `index.ts` | Re-exports |

## Core API

### Registering providers

```ts
import { createDetectProviderRegistry, createDetectResolver } from '@owliabot/ais-ts-sdk';

const registry = createDetectProviderRegistry();
registry.register('best_quote', 'my-provider', async (detect, ctx) => {
  // Use detect.constraints / ctx.runtime / external IO as needed
  return { /* ... */ };
});

const detect = createDetectResolver(registry);
```

### Using in ValueRef evaluation

```ts
import { evaluateValueRefAsync } from '@owliabot/ais-ts-sdk';

await evaluateValueRefAsync(
  { detect: { kind: 'best_quote', provider: 'my-provider', constraints: { /* ... */ } } },
  resolverContext,
  { detect }
);
```

## Default providers

The SDK ships with a minimal built-in provider:
- `choose_one/builtin` — deterministic: picks the first candidate

