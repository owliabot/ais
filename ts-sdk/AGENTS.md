# AGENTS.md — AIS TypeScript SDK

Guidelines for AI agents working on this codebase.

## Project Structure

```
ts-sdk/
├── src/
│   ├── schema/      # Zod schemas + TS types (see src/schema/README.md)
│   ├── resolver/    # Reference + expression resolution (see src/resolver/README.md)
│   ├── validator/   # Constraint + workflow validation (see src/validator/README.md)
│   ├── execution/   # Transaction building + ABI encoding (see src/execution/README.md)
│   ├── registry/    # Registry helpers (JCS canonicalization) (see src/registry/README.md)
│   ├── detect/      # Detect provider registry (see src/detect/README.md)
│   ├── cel/         # CEL expression parser/evaluator (see src/cel/README.md)
│   ├── builder/     # Fluent DSL for building AIS docs (see src/builder/README.md)
│   ├── cli/         # Command-line tools (see src/cli/README.md)
│   ├── parser.ts    # YAML parsing + document detection
│   ├── loader.ts    # File/directory loading utilities
│   └── index.ts     # Public API exports
├── tests/           # Test files
└── package.json
```

## ⚠️ README Sync Rule

**When you modify any file in `src/<module>/`, you MUST update the corresponding `src/<module>/README.md`.**

This includes:
- Adding new functions/types → document in README
- Changing function signatures → update API section
- Adding new files → add to file structure table
- Removing functionality → remove from README

Each README contains:
1. Module overview
2. File structure table
3. Core API documentation
4. Usage examples
5. Type definitions
6. Implementation notes

## Module Dependencies

```
schema ← (base, no deps)
    ↑
resolver ← schema
    ↑
validator ← schema, resolver
    ↑
execution ← schema, resolver, cel
    ↑
cel ← (standalone)
    ↑
builder ← schema
    ↑
cli ← schema, loader, parser
```

## Key Design Decisions

1. **Use standard chain SDKs**: EVM ABI encoding/decoding uses `ethers`; Solana helpers use `@solana/web3.js` / `@solana/spl-token`.
2. **Zod for schemas**: All types inferred from Zod schemas via `z.infer<>`
3. **CEL for expressions**: Custom parser, not Google's heavy implementation
4. **Fluent builders**: Chainable API for programmatic document construction

## Testing

```bash
npm test           # Run all tests
npm run test:watch # Watch mode
```

Tests live in `tests/` and mirror `src/` structure.

## Build

```bash
npm run build      # TypeScript → dist/
npm run lint       # ESLint
npm run typecheck  # tsc --noEmit
```

## Common Tasks

### Adding a new schema field

1. Update `src/schema/<document>.ts`
2. Update `src/schema/README.md`
3. Add test cases
4. Update builder if needed (`src/builder/<document>.ts`)

### Adding a new lint rule

1. Add rule object to `src/cli/commands/lint.ts`
2. Update `src/cli/README.md` with rule description
3. Add test case

### Adding a CEL builtin function

1. Add to `BUILTINS` in `src/cel/evaluator.ts`
2. Update `src/cel/README.md` function table
3. Add test case

## Commit Guidelines

- Keep commits focused (one logical change)
- Run `npm test` before committing
- Update READMEs in the same commit as code changes
