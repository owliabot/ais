# Plugins Module (AIS 0.0.2)

Minimal plugin system for extending AIS without bloating the core spec.

Core goal:
- AIS core stays small (EVM + Solana + BTC PSBT + Composite).
- All other chains/protocol-specific execution types are supported via **plugin registration**.

## Execution type plugins

An execution type plugin registers:
- `type`: execution `type` string
- `schema`: Zod schema for validating that execution spec
- optional hooks for planning/readiness

The parser enforces:
- Core execution types are always validated by the core schemas.
- Non-core execution types MUST be registered before parsing; otherwise parsing fails with a clear error.

## API

```ts
import { createExecutionTypeRegistry, registerExecutionType } from '@owliabot/ais-ts-sdk';
```

## Files

| File | Purpose |
|------|---------|
| `execution.ts` | Execution type registry + validation helpers |
