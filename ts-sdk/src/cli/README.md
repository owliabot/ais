# CLI Module

Command-line tool for validating, linting, and checking AIS files (AIS `0.0.2`).

## File Structure

| File | Purpose |
|------|---------|
| `index.ts` | CLI entry point, argument parser, command router |
| `utils.ts` | Shared utilities: file collection, result formatting, colors |
| `commands/validate.ts` | Schema validation against Zod schemas |
| `commands/lint.ts` | Best-practice linting (uses `validator.lintDocument`) |
| `commands/check.ts` | Combined schema + workspace + workflow reference checks |

## Usage

```bash
# Validate AIS files against schema
ais validate ./protocols/
ais validate spec.ais.yaml

# Lint for best practices
ais lint ./specs/*.ais.yaml

# Run all checks (validate + lint)
ais check . --recursive

# JSON output for CI/CD
ais validate protocol.ais.yaml --json

# Quiet mode (errors only)
ais check . -q
```

## Commands

### `validate`

Validates AIS files against their Zod schemas. Checks:
- YAML syntax
- Required fields
- Type correctness
- Schema-specific constraints

```bash
ais validate <path...>
```

### `lint`

Checks for best practices and common issues:

**Protocol rules:**
- `protocol-has-description` (warning) — Protocol should have description
- `protocol-has-deployments` (error) — Must have ≥1 deployment
- `action-has-description` (info) — Actions should be documented
- `action-has-params` (warning) — Actions should define params
- `contract-address-format` (error) — Valid 0x addresses

**Pack rules:**
- `pack-has-description` (warning) — Pack should have description
- `pack-has-includes` (error) — Must include ≥1 protocol
- `pack-skill-ref-format` (warning) — Skill includes should specify a version

**Workflow rules:**
- `workflow-has-description` (warning) — Workflow should have description
- `workflow-has-nodes` (error) — Must have ≥1 node
- `workflow-node-has-id` (error) — Nodes must have ids (and ids must be unique)
- `workflow-node-skill-ref-format` (warning) — Node skill refs should include version

```bash
ais lint <path...>
```

### `check`

Runs cross-file and reference checks for a workspace directory:
- Schema validity (YAML + Zod)
- Cross-file references (workflow → pack → protocol)
- Workflow reference validation (actions/queries, deps, refs)

```bash
ais check <path...>
```

## Options

| Flag | Description |
|------|-------------|
| `-r, --recursive` | Process directories recursively (default: true) |
| `-q, --quiet` | Only show errors |
| `-v, --verbose` | Show all details including successes |
| `--json` | Output results as JSON |
| `--no-color` | Disable colored output |

## Output Formats

### Text (default)

```
Validation Results
──────────────────────────────────────────────────

./protocols/uniswap-v3.ais.yaml
  ✓ Schema validation passed

./protocols/aave-v3.ais.yaml
  ✗ Missing required field: meta.version
  ⚠ Action 'supply' has no description

──────────────────────────────────────────────────
Summary: 1 passed, 1 warnings, 1 errors
```

### JSON (`--json`)

```json
{
  "title": "Validation",
  "summary": {
    "total": 2,
    "success": 1,
    "errors": 1,
    "warnings": 1,
    "info": 0
  },
  "results": [
    {
      "path": "./protocols/uniswap-v3.ais.yaml",
      "type": "success",
      "message": "Schema validation passed"
    }
  ]
}
```

## File Detection

The CLI recognizes AIS files by extension:
- `.ais.yaml` / `.ais.yml` — Protocol Specs
- `.ais-pack.yaml` / `.ais-pack.yml` — Packs
- `.ais-flow.yaml` / `.ais-flow.yml` — Workflows

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | All checks passed |
| 1 | One or more errors found |

## Implementation Notes

- **Zod-based validation**: Uses the same schemas as the SDK
- **Extensible rules**: Lint rules are pluggable via the validator registry (see `validator/plugins.ts`)
- **TTY-aware**: Colors auto-disabled for piped output
- **Recursive by default**: Processes subdirectories unless `--no-recursive`

## Dependencies

- `schema/` — Zod schemas for validation
- `loader.ts` — File loading utilities
- `parser.ts` — YAML parsing
