# AIS-1E: Executable Candidates (`ais-executable-candidates/0.0.1`)

Status: Draft  
Spec Version: 0.0.2

Executable Candidates is an agent-facing, cacheable output format that answers:

- “Under this **pack** (policy boundary) and this **engine capability set**, what actions/queries/plugins are executable?”

This prevents agents from:

- ingesting full protocol specs, and
- planning actions that will be rejected by allowlists/capabilities at runtime.

It complements:

- Catalog cards: `specs/ais-1-catalog.md` (`ais-catalog/0.0.1`)
- Capabilities: `specs/ais-1-capabilities.md`
- Packs: `specs/ais-1-pack.md`
- Intent contract: `specs/ais-2-agent-intent.md`
- Segmented planning tools: `specs/ais-2-agent-planning.md`

---

## 1. Top-level shape

An executable candidates document is a single JSON object:

- `schema`: `"ais-executable-candidates/0.0.1"`
- `created_at?`: RFC3339 timestamp (informational; ignored for hashing)
- `hash`: sha256 over the normalized content (see §3)
- `catalog_schema`: e.g. `"ais-catalog/0.0.1"`
- `catalog_hash`: hash of the source catalog
- `pack?`: pack identity summary `{ name, version }`
- `chain_scope?`: optional chain scope applied by the host
- `actions`: `ActionCard[]` (index cards)
- `queries`: `QueryCard[]` (index cards)
- `execution_plugins`: `ExecutionPluginCandidate[]` (exploded; see §2)
- `extensions?`: extension slot

Notes:

- `actions/queries` MUST be stable-sorted by `ref`.
- `execution_plugins` are “exploded” to make selection easier for agents.
- `actions/queries` in this document are index cards only; detail payload is fetched separately by `ref` (outside this schema).

---

## 2. Provider/plugin candidate explosion (normative)

### 2.1 Execution plugins

Similarly, plugin execution allowlists are exploded into:

- either `{ type, chain }`, or
- `{ type }` when chain is not specified.

---

## 3. Determinism and hashing (normative)

`hash` MUST be:

- `sha256` of stable JSON encoding of the document with:
  - `created_at` removed (ignored),
  - `hash` removed (ignored)

Stable JSON encoding:

- sort object keys lexicographically
- preserve array order
- compact JSON output

Output format:

- lowercase hex string (no `0x` prefix)

---

## 4. Authority schema

- JSON Schema: `schemas/0.0.2/executable-candidates.schema.json`

---

## 5. Planner tool-calling contract (normative)

For intent-mode planning, hosts SHOULD expose the following planner tools to LLM:

### 5.1 `list_candidates`

Purpose:

- return one `ais-executable-candidates/0.0.1` snapshot for current workspace + pack + chain scope.

Input:

- empty object (`{}`), or implementation-private filters under host-controlled extensions.

Output:

- single object with `schema="ais-executable-candidates/0.0.1"`.

Rules:

- host MUST treat this output as immutable within one planner round.
- host SHOULD include `hash` so planner can reference the snapshot identity.

### 5.2 `get_candidate_detail`

Purpose:

- fetch detail cards by `ref` for a small subset selected from index candidates.

Input:

- `{ "refs": string[] }`

Output:

- `{ "items": CatalogCard[] }` where all returned cards match requested `refs`.

Rules:

- host MUST ignore unknown refs (do not fail whole call).
- host SHOULD cap `refs` count per call to control token usage.

### 5.3 Determinism boundary

- `list_candidates` and `get_candidate_detail` responses MUST be deterministic for the same input within a single planning turn.
- if host state changes across turns (for example pack update), host SHOULD surface a new `hash` and planners MUST treat it as a new snapshot.

### 5.4 Compact/detail guidance for segmented planning

In segmented planning mode (`plan.begin/propose_segment/revise_segment`):

- host SHOULD return compact index cards in `list_candidates` by default.
- planner SHOULD use `get_candidate_detail` lazily and only for refs selected in the current segment.
- host SHOULD enforce detail ref count and response size limits to protect token budget.

These limits are host policy; planners MUST tolerate truncated or partial detail responses.
