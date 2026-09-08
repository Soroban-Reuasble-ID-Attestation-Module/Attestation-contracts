# Contract Specification

The normative specification of the attestation contract. SDKs, the backend
indexer, the frontend, and downstream contracts (escrow, gating) are built
against this document. Anything not specified here is an implementation detail
and must not be relied upon.

## 1. Functions

### `__constructor(admin: Address)`

Binds the admin. Called once at deployment; reverts if the admin's
authorization cannot be satisfied. Emits `contract_initialized`.

### `add_issuer(issuer: Address) -> Result<(), AttestationError>`

Registers `issuer`. **Admin only.** Errors: `NotInitialized`, `Unauthorized`
(auth failure), `InvalidIssuer` (already registered). Emits `issuer_added`.

### `remove_issuer(issuer: Address) -> Result<(), AttestationError>`

Removes `issuer` from the registry. **Admin only.** Existing attestations are
unaffected until revoked or expired. Errors: `NotInitialized`,
`Unauthorized`, `InvalidIssuer` (not registered). Emits `issuer_removed`.

### `is_issuer(address: Address) -> bool`

`true` if `address` is currently registered. Total function.

### `issue_attestation(
    issuer: Address,
    subject: Address,
    claim_type: Symbol,
    claim_hash: BytesN<32>,
    expiry: u64,
) -> Result<u32, AttestationError>`

Issues an attestation and returns its id.

- **Auth:** `issuer.require_auth()` and `issuer` must be registered.
- **Validation:**
  - `claim_hash` must not be all-zero bytes (`InvalidClaim`).
  - `expiry` must be strictly greater than the current ledger timestamp
    (`InvalidExpiry`).
  - No *active* attestation may exist for `(subject, claim_type)`
    (`AlreadyIssued`); a previously **revoked** attestation may be replaced.
- **Effects:** writes `Attestation(id)`, updates `SubjectIndex`, bumps
  `NextId`, extends TTLs, emits `attestation_issued` (topic: `id`).
- Errors: `NotInitialized`, `Unauthorized`, `InvalidClaim`, `InvalidExpiry`,
  `AlreadyIssued`, `NotFound` (invariant violation).

### `revoke(caller: Address, attestation_id: u32) -> Result<(), AttestationError>`

Marks an attestation revoked. **The issuing issuer or the admin only.**
Irreversible. Errors: `NotInitialized`, `Unauthorized`, `NotFound`,
`Revoked` (already revoked). Emits `attestation_revoked` (topic: `id`).

### `verify(subject: Address, claim_type: Symbol) -> bool`

`true` iff an attestation exists for the exact `(subject, claim_type)` pair,
is not revoked, and is not expired (`expiry > now`). **Total function** —
returns `false` in every failure case, never panics, requires no
authorization. Safe for cross-contract calls.

### `get_attestation(attestation_id: u32) -> Result<Attestation, AttestationError>`

Returns the full record: `id`, `subject`, `claim_type`, `claim_hash`
(commitment only), `issuer`, `issued_at`, `expiry`, `revoked`. Errors:
`NotFound`.

### `verify_claim_commitment(
    subject: Address,
    claim_type: Symbol,
    claim_value: Bytes,
    salt: Bytes,
) -> bool`

Selective disclosure. Recomputes `sha256(claim_value ‖ salt)` and returns
`true` iff it matches the stored commitment **and** the attestation is active
(not revoked, not expired). Total function.

## 2. Types

### `Attestation`

| Field | Type | Semantics |
|---|---|---|
| `id` | `u32` | Monotonic, never reused |
| `subject` | `Address` | The Stellar address the attestation is bound to |
| `claim_type` | `Symbol` | e.g. `kyc_verified`, `accredited_investor` |
| `claim_hash` | `BytesN<32>` | `sha256(claim_value ‖ salt)` |
| `issuer` | `Address` | Registered issuer that issued it |
| `issued_at` | `u64` | Ledger timestamp (Unix seconds) |
| `expiry` | `u64` | Unix seconds; 0 is not special — must be future at issue |
| `revoked` | `bool` | Persistent revocation flag |

## 3. Error codes (stable)

| Code | Name | Meaning |
|---|---|---|
| 1 | `Unauthorized` | Caller lacks permission for this operation |
| 2 | `NotFound` | No such attestation / index entry |
| 3 | `Expired` | Attestation is expired |
| 4 | `Revoked` | Attestation is revoked (or already revoked) |
| 5 | `InvalidExpiry` | Expiry not strictly in the future |
| 6 | `InvalidSubject` | Subject is not a valid identity |
| 7 | `InvalidIssuer` | Not a registered issuer (or already registered) |
| 8 | `InvalidClaim` | Claim commitment is invalid (e.g. all-zero) |
| 9 | `AlreadyIssued` | Active attestation already exists for the pair |
| 10 | `NotInitialized` | Contract state missing (defensive; constructor makes this unreachable) |
| 11 | `AlreadyInitialized` | Reserved for interface stability |

## 4. Events

| Event | Topic 1 | Topic 2 | Data (map) |
|---|---|---|---|
| `contract_initialized` | `"contract_initialized"` | — | `admin` |
| `issuer_added` | `"issuer_added"` | — | `issuer` |
| `issuer_removed` | `"issuer_removed"` | — | `issuer` |
| `attestation_issued` | `"attestation_issued"` | `id: u32` | `subject`, `claim_type`, `issuer`, `expiry` |
| `attestation_revoked` | `"attestation_revoked"` | `id: u32` | `revoker` |

Data payloads are maps keyed by the field names above (values are the
corresponding SCVal types). Indexers must filter by topic 1 (event name) and
may use topic 2 (id) for per-attestation queries.

## 5. Commitment scheme

```
claim_hash = sha256(claim_value ‖ salt)
```

- Issuers compute the digest off-chain (SDK helper: `computeClaimHash`).
- `verify_claim_commitment` recomputes it in the contract.
- Salt requirements: unique per claim, ≥ 16 random bytes, kept secret by the
  subject. See `SECURITY.md` §2.9 for the guessing-threat analysis.

## 6. Versioning

The interface in this document is stable for the `v0.1` line. Adding
functions is backward compatible; changing signatures, error codes, event
schemas, or the commitment scheme is a **breaking change** and must be
accompanied by a new contract version and a migration path for consumers.