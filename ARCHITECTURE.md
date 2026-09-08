# Architecture

This document describes how the attestation contract is built: storage model,
TTL strategy, event flow, authorization model, and the contract-to-contract
story. It is written for maintainers and for reviewers who want to understand
the design decisions.

## 1. Overview

```
┌──────────────┐   issue/revoke (auth)   ┌─────────────────────┐
│   Issuer     │ ───────────────────────▶ │                     │
└──────────────┘                          │  Attestation        │
                                          │  Contract           │
┌──────────────┐   verify / get /        │  (persistent state) │
│  Subject     │   verify_claim_commit   │                     │
└──────────────┘ ───────────────────────▶ │                     │
                                          └──────────┬──────────┘
┌──────────────┐   verify() cross-contract            │
│  Consumer    │ ─────────────────────────────────────▶
│  (escrow…)   │   (no auth required; total function)  │
└──────────────┘                                       ▼
                                              return bool
```

The contract is stateless *in the service sense* — all state lives in Soroban
ledger storage. There is no off-chain database, no external dependency, and no
oracle. Verification is a pure function of ledger state and the current ledger
timestamp.

## 2. Storage model

Two Soroban storage regions are used:

### Instance storage (single entry, cheap)

| Key | Value | Purpose |
|---|---|---|
| `Admin` | `Address` | Trust root that manages the issuer registry |
| `NextId` | `u32` | Monotonic attestation-id counter |

Instance storage is read on every entrypoint, so its TTL is extended on every
write (`extend_instance_ttl`).

### Persistent storage (per-record)

| Key | Value | Purpose |
|---|---|---|
| `Issuer(Address)` | `()` | Presence marker: address is a registered issuer |
| `Attestation(u32)` | `Attestation` | The attestation record |
| `SubjectIndex(Address, Symbol)` | `u32` | `(subject, claim_type)` → attestation id |

### The `(subject, claim_type)` uniqueness index

`SubjectIndex` is the load-bearing piece of the design:

- It makes `verify(subject, claim_type)` an **O(1) lookup** — no scans.
- It enforces **one active attestation per pair** at the storage level, so an
  attacker cannot stack attestations or shadow a revoked one.
- Re-issuance after revocation simply points the index at the new id; the old
  (revoked) record is retained under its own `Attestation(id)` key for audit.

## 3. TTL management

Every Soroban entry has a TTL in ledgers (ledger ≈ 5 s on mainnet/testnet,
17,280 ledgers/day). The strategy is *extend on touch*:

- `storage.rs` defines two windows: **31 days** for instance storage,
  **365 days** for persistent records.
- Every write and every read of a persistent entry calls
  `extend_persistent_ttl`, which extends to 365 days whenever the remaining
  TTL drops below half the window.
- Frequently-verified attestations therefore stay live indefinitely, while
  abandoned state is eventually reaped — no manual TTL upkeep.

## 4. Authorization model

| Operation | Who may call | Enforcement |
|---|---|---|
| `add_issuer` / `remove_issuer` | The stored `admin` | `admin.require_auth()` |
| `issue_attestation` | A registered issuer | `issuer.require_auth()` + registry presence check |
| `revoke` | The issuing issuer **or** the admin | `caller.require_auth()` + equality check |
| `verify`, `get_attestation`, `is_issuer`, `verify_claim_commitment` | Anyone | none (total functions) |

Notes:

- The admin can never *issue* attestations unless also registered as an issuer
  (separation of powers: registry management ≠ issuance).
- `revoke` is deliberately restricted to the issuer who issued the record (or
  the admin) — a stranger can never revoke someone else's attestation.
- Authorization is bound to the transaction by Soroban's auth framework, so
  the *caller identity* cannot be spoofed by passing a different address.

## 5. Constructor, not `initialize`

Admin binding happens in `__constructor` rather than a callable `initialize`:

- **No front-running.** A deploy-then-initialize pattern lets an attacker call
  `initialize` first and become admin. Constructor args are part of the deploy
  transaction, so the admin is fixed atomically.
- **No "uninitialized" state** exists in practice, which removes an entire
  class of misconfiguration.

## 6. Commitment scheme

```
claim_hash = sha256(claim_value ‖ salt)
```

- Issuance stores `claim_hash` only.
- `verify_claim_commitment` recomputes `sha256(claim_value ‖ salt)` from the
  caller-supplied value and salt, and compares.
- The scheme is intentionally simple and documented so that the TypeScript SDK,
  the Python SDK, and the frontend all compute identical commitments.
- Salt guidance: 16+ random bytes per claim, never reused across claims of the
  same value; treat the salt as secret unless disclosure of the value is fine.

> ⚠️ Because the preimage is a plain concatenation, two different
> `(value, salt)` splits could theoretically collide in the *concatenation*
> space. In practice this is not exploitable because the issuer controls both
> fields at issuance, but integrators who need domain separation can prefix
> values with a length tag before hashing — the contract compares digests, so
> any agreed encoding works as long as issuer and prover use the same one.

## 7. Event flow

Events are the integration point for the [backend indexer]
(../Attestation-backend-sdk) and the frontend.

```
issuer_added / issuer_removed     → update issuer registry cache
attestation_issued (topic: id)    → upsert attestation index row
attestation_revoked (topic: id)   → set revoked flag in index row
```

- The attestation id is published as an **event topic** so the indexer can
  filter by topic without decoding data.
- Data payloads are self-describing maps (`subject`, `claim_type`, `issuer`,
  `expiry`, `revoker`), so consumers decode them from the schema alone.

## 8. Contract-to-contract usage

`verify(subject, claim_type)` is designed to be called from other contracts:

- It is a **total function** — it never panics, never requires auth, and never
  emits diagnostics on failure paths.
- It returns plain `bool`, so a calling contract can use it directly in a
  gate (`if !attestation.verify(subject, claim_type) { return Err(...) }`).
- It reads ledger state only; there are no side effects to reason about.
- The [integration suite](tests/integration.rs) proves the pattern with a
  `ConsumerContract` that gates on `verify()` and observes revocation and
  expiry propagation.

The escrow demonstration in the frontend repository relies on exactly this
property: **the escrow contract itself** calls `verify()`, so a compromised or
bypassed frontend can never release funds — the gate is on-chain.

## 9. Read-path cost profile

All entrypoints are constant-time with respect to the number of attestations:

- `verify`: 1 index read + 1 record read (both extend-on-touch).
- `get_attestation`: 1 record read.
- `verify_claim_commitment`: 2 reads + 1 SHA-256 over the supplied value.
- Mutations: fixed reads/writes + 1–2 auth checks.

No iteration over subjects, claim types, or issuers ever occurs, so fees are
bounded regardless of registry size.