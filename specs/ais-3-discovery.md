# AIS-3D: Discovery & Install Inputs — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document defines the normative input model for dynamic protocol discovery/installation.

## 1. `ProtocolSource` (normative)

Any dynamic protocol install attempt MUST declare a `ProtocolSource` with `kind`:

- `local_path`
- `registry_ref`
- `remote_url`
- `llm_generated`

### 1.1 `local_path`

Required fields:

- `kind = "local_path"`
- `path` (workspace-relative path)

Rules:

- host MUST reject path traversal outside workspace root.
- source is treated as immutable snapshot content for hashing/audit.

### 1.2 `registry_ref`

Required fields:

- `kind = "registry_ref"`
- `registry` (registry id)
- `package` (protocol/package name)
- `version` (immutable version string)
- `digest_sha256` (expected content digest)

Optional fields:

- `signature`
- `publisher`

Rules:

- host MUST pin by digest; latest-pointer only resolution is not sufficient.
- if pack requires signature, missing/invalid signature MUST be hard-blocked.

### 1.3 `remote_url`

Required fields:

- `kind = "remote_url"`
- `url` (https only)
- `digest_sha256`

Optional fields:

- `signature`
- `publisher`

Rules:

- domain MUST satisfy pack/domain allowlist when active.
- install MUST verify fetched content digest before use.

### 1.4 `llm_generated`

Required fields:

- `kind = "llm_generated"`
- `generator_id`
- `prompt_summary`
- `content_digest_sha256`

Optional fields:

- `model`
- `conversation_hash`

Rules:

- generated spec MUST be materialized as a local file first, then treated as `local_path` for execution pipeline.
- install record MUST include generation provenance summary.

## 2. Installation step is policy-gated (normative)

Dynamic install is an explicit pre-planning step:

1) source resolve/fetch  
2) policy decision (`ok|need_user_confirm|hard_block`)  
3) on allow: persist protocol artifact locally and register into resolver context

Engine execution MUST NOT perform implicit network installs bypassing policy gate.

## 3. Safe defaults (recommended)

- `safe`: allow only `local_path` and `registry_ref` with pinned digest.
- `assist`: allow `registry_ref` by default; remote/llm-generated requires explicit confirm.
- `yolo`: may allow all configured sources, but still auditable and still constrained by handler registration/pack allowlists.

Core install-policy knobs (v3 minimal):

- `mode`
- `allowed_sources`
- `require_signature`

Other source controls (for example domain/registry/publisher allowlists) SHOULD live in host-specific `extensions` and are outside the core decision contract.
