# Registry Module

Helpers related to AIS registry semantics (AIS `0.0.2`).

## File Structure

| File | Purpose |
|------|---------|
| `jcs.ts` | RFC 8785-like JSON canonicalization (JCS) + `specHash` helper |
| `index.ts` | Re-exports |

## Core API

### `canonicalizeJcs()`

Canonicalize a JSON-like value into a stable string:
- sorts object keys lexicographically
- preserves array order
- rejects non-JSON values (`undefined`, non-finite numbers, `bigint`, `Uint8Array`)

```ts
import { canonicalizeJcs } from '@owliabot/ais-ts-sdk';

canonicalizeJcs({ b: 1, a: 2 });
// {"a":2,"b":1}
```

### `specHashKeccak256()`

Convenience helper for AIS `specHash`:

```ts
import { specHashKeccak256 } from '@owliabot/ais-ts-sdk';

const specHash = specHashKeccak256(specObject);
// 0x...
```

## Notes

- The spec recommends canonical JSON as the hash input for `specHash` to avoid YAML formatting differences.
- Hash algorithm is registry-defined; keccak256 is provided as a common default.

