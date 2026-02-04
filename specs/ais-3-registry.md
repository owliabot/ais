# AIS-3: Registry & Discovery

> Status: Draft
> Version: 1.0.0

## Abstract

AIS-3 defines the on-chain registry contract and discovery layer for publishing, verifying, and discovering AIS specs. The registry ensures agents consume tamper-proof, verified protocol definitions.

---

## Deployment

- **Chain:** Base (eip155:8453)
- **Rationale:** Low gas, EVM-compatible, widely adopted L2

---

## Contract: `AISRegistry`

### Storage

```solidity
struct Skill {
    address   owner;           // Registrant address (protocol team or maintainer)
    string    protocol;        // Protocol ID (e.g., "uniswap-v3")
    string    version;         // Semver (e.g., "1.2.0")
    string    specURI;         // IPFS or Arweave URI to full spec YAML
    bytes32   specHash;        // keccak256 of spec content
    uint64    chainScope;      // Bitmask of supported chain families
    uint64    registeredAt;    // Block timestamp
    uint64    updatedAt;       // Block timestamp of last update
    bool      verified;        // Approved by verifier(s)
    bool      deprecated;      // Marked as end-of-life
}

mapping(bytes32 => Skill) public skills;         // skillId => Skill
mapping(string => bytes32) public latestByName;  // protocol name => latest skillId
bytes32[] public allSkillIds;
```

**skillId** = `keccak256(abi.encodePacked(owner, protocol, version))`

### Chain Scope Bitmask

| Bit | Chain Family |
|-----|--------------|
| 0   | EVM (all eip155) |
| 1   | Solana |
| 2   | Cosmos |
| 3   | Bitcoin |
| 4   | Aptos |
| 5   | Sui |
| 6-63 | Reserved |

Example: EVM + Solana = `0b011` = `3`

### Functions

```solidity
// ─── Registration ────────────────────────

/// Register a new skill spec.
function register(
    string calldata protocol,
    string calldata version,
    string calldata specURI,
    bytes32 specHash,
    uint64  chainScope
) external returns (bytes32 skillId);

/// Update an existing skill (owner only). Resets verified to false.
function update(
    bytes32 skillId,
    string calldata version,
    string calldata specURI,
    bytes32 specHash,
    uint64  chainScope
) external;

/// Mark a skill as deprecated (owner or verifier).
function deprecate(bytes32 skillId) external;

/// Transfer ownership of a skill.
function transferOwnership(bytes32 skillId, address newOwner) external;


// ─── Verification ────────────────────────

/// Verify a skill (verifier only).
function verify(bytes32 skillId) external;

/// Revoke verification (verifier only).
function unverify(bytes32 skillId) external;


// ─── Queries ─────────────────────────────

function getSkill(bytes32 skillId) external view returns (Skill memory);
function getLatest(string calldata protocol) external view returns (Skill memory);
function listByChain(uint64 chainBit) external view returns (bytes32[] memory);
function listVerified() external view returns (bytes32[] memory);
function listVerifiedPaginated(uint256 offset, uint256 limit) 
    external view returns (bytes32[] memory, uint256 total);
function isVerified(bytes32 skillId) external view returns (bool);
function skillCount() external view returns (uint256);


// ─── Admin ───────────────────────────────

function addVerifier(address verifier) external;
function removeVerifier(address verifier) external;
function transferAdmin(address newAdmin) external;
```

### Events

```solidity
event SkillRegistered(bytes32 indexed skillId, address indexed owner, string protocol, string version);
event SkillUpdated(bytes32 indexed skillId, string version, bytes32 specHash);
event SkillVerified(bytes32 indexed skillId, address indexed verifier);
event SkillUnverified(bytes32 indexed skillId, address indexed verifier);
event SkillDeprecated(bytes32 indexed skillId);
event OwnershipTransferred(bytes32 indexed skillId, address indexed oldOwner, address indexed newOwner);
```

---

## Discovery Layer

### Requirements

AIS-1.0 specifies these discovery semantics for consuming clients:

1. **Pagination Required** — Discovery interfaces MUST support pagination
   ```
   GET /skills?offset=0&limit=50
   Response: { skills: [...], total: 150, hasMore: true }
   ```

2. **Verified by Default** — Clients SHOULD load only `verified` skills by default
   ```
   GET /skills?verified=true  (default)
   GET /skills?verified=false (opt-in for unverified)
   ```

3. **Community Packages** — Unverified specs enter a "community pack" group
   - Deployers must explicitly enable via Pack configuration
   - UI should clearly distinguish verified vs community

4. **Graceful Degradation** — On spec fetch failure:
   - Use last known verified version from cache
   - Log warning but don't fail completely
   - Retry with exponential backoff

5. **Domain Verification** — For authority binding:
   - Protocol places verification file at: `https://<domain>/.well-known/ais-verify.json`
   - Resolves namespace conflicts and impersonation

### Discovery Endpoints (Reference)

```
GET  /skills                    # List all (paginated)
GET  /skills?chain=eip155       # Filter by chain family
GET  /skills?protocol=uniswap-v3  # Filter by protocol name
GET  /skills/{skillId}          # Get specific skill
GET  /skills/{skillId}/spec     # Fetch and validate spec content
POST /skills/{skillId}/refresh  # Force re-fetch from IPFS
```

### Caching Strategy

| Layer | TTL | Invalidation |
|-------|-----|--------------|
| Registry index | 6 hours | On SkillUpdated event |
| Spec content | 24 hours | On specHash change |
| Query results | Per cache_ttl | Time-based |

---

## Governance Evolution

### Phase A: Centralized (Launch)

```
Admin = single EOA (deployer)
Verifiers = [deployer]

- Admin registers & verifies all skills
- No external submissions
- Focus: bootstrap Top 20 protocols
```

### Phase B: Open Submissions + Domain Verification

```
Admin = 2-of-3 multisig
Verifiers = [multisig members]

Submission flow:
1. Protocol team calls register() → skill created, verified=false
2. Protocol places verification file at:
   https://<protocol-domain>/.well-known/ais-verify.json
   {
     "registry": "0x<registry_address>",
     "skillId": "0x<skill_id>",
     "owner": "0x<registrant_address>"
   }
3. Verifier checks domain ownership + spec accuracy
4. Verifier calls verify()

Update flow:
1. Owner calls update() → verified resets to false
2. Re-verification required
3. Agents continue using last verified version until new one passes
```

### Phase C: Decentralized Governance (Future)

```
Admin = Governor contract
Verifiers = staker set

- Stake required to register (e.g., 0.01 ETH)
- Community can challenge unverified/malicious specs
- Successful challenge → slash stake
- Verified specs earn reputation score
- Optional governance token for voting
```

---

## Agent Consumption Flow

```
Wallet Engine startup:
  1. Read AISRegistry.listVerifiedPaginated() with pagination
  2. For each verified skill:
     a. Fetch specURI from IPFS/Arweave
     b. Compute keccak256(content), compare with on-chain specHash
     c. If match → parse and cache locally
     d. If mismatch → skip, log warning, use cached version if available
  3. Set up periodic sync (e.g., every 6 hours)
  4. Subscribe to registry events for real-time updates

Agent request flow:
  GET /skills → engine returns cached verified skills
  POST /execute { skill: "uniswap-v3", action: "swap", params: {...} }
    → Engine looks up cached spec
    → Validate capabilities_required
    → Execute requires_queries
    → Build transaction per execution spec
    → Policy check → sign → broadcast
```

---

## Security Considerations

1. **Spec content is verified off-chain** — The contract only stores the hash. Verifiers must check the actual YAML content.

2. **IPFS pinning** — Spec authors should pin their content. Registry could run a dedicated IPFS node as backup.

3. **Version immutability** — Once a (protocol, version) pair is registered, updating changes the version. Old versions remain queryable.

4. **Frontrunning** — Registration is permissionless; someone could register "uniswap-v3" before the real team. Mitigation: domain verification process.

5. **Contract upgradability** — Use UUPS proxy pattern for Phase A/B. Lock upgradeability in Phase C.

6. **Spec fetch failures** — Always fall back to last known verified version. Never execute with unverified/unfetched specs.

---

## Gas Estimates

| Operation | Estimated Gas |
|-----------|---------------|
| register() | ~150,000 |
| update() | ~80,000 |
| verify() | ~30,000 |
| getSkill() | View (free) |
| listVerifiedPaginated() | View (free) |

At Base gas prices (~0.01 gwei), registration costs < $0.01.
