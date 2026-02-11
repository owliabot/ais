# AIS-3R: Registry Semantics — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document specifies registry semantics (not a specific implementation).

## 1. Identity model (D5=A)

AIS 0.0.2 defines:

- `protocolId = keccak256(owner, protocol)` (version is NOT part of protocolId)
- `version` is a mutable field of a protocol record, updated by the owner.

Rationale:
- Avoids semantic contradictions where `update(protocolId, version, ...)` changes version but protocolId “should have changed”.

## 2. `specHash`

`specHash` MUST be computed over a **canonical byte representation** of the spec.

Recommendation:
- Engines/registries SHOULD hash **canonical JSON** obtained by parsing YAML into a data model and re-serializing with a stable canonicalizer.
- Hashing raw YAML bytes is NOT recommended due to non-canonical formatting differences.

## 3. Latest pointers

Registry MUST provide:

- `latestByName[protocol] -> protocolId` (or record)
- a method to fetch the latest verified version
