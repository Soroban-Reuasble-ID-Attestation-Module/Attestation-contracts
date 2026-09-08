# Attestation Contracts

A production-grade, reusable **Stellar identity & attestation protocol** built on
Soroban. Authorized issuers issue cryptographic attestations bound to Stellar
`Address` subjects; anyone can verify them; attestations can be revoked;
verifiers can prove specific claims without revealing the underlying data
(selective disclosure).

**Never store raw KYC/PII. Only cryptographic commitments live on-chain.**

| | |
|---|---|
| Network | Stellar (Soroban), Protocol 23+ / soroban-sdk 27 |
| Language | Rust (`#![no_std]`), compiled to `wasm32v1-none` |
| Testnet deploy | [`CB2MGYTG6MIIDYWVB5BV4FLEF7KRDEF556JMVZXSZ22XALB7MUC7LU2S`](https://stellar.expert/explorer/testnet/contract/CB2MGYTG6MIIDYWVB5BV4FLEF7KRDEF556JMVZXSZ22XALB7MUC7LU2S) |
| CI | fmt + clippy (`-D warnings`) + 31 tests + release wasm build |
| License | Apache-2.0 |

## Table of contents

- [Why this exists](#why-this-exists)
- [Core concepts](#core-concepts)
- [Contract interface](#contract-interface)
- [Repository layout](#repository-layout)
- [Getting started](#getting-started)
- [Build, test, deploy](#build-test-deploy)
- [Deployed testnet addresses](#deployed-testnet-addresses)
- [Documentation](#documentation)
- [Security](#security)

## Why this exists

Attestations (KYC verification, accreditation, membership, age proofs, …) are
currently siloed: every issuer builds its own storage, its own verification,
its own revocation, and there is no portable way for a downstream contract to
trust *any* issuer's attestations. This protocol defines a single, minimal,
on-chain standard so that:

- **Issuers** register once and issue commitments, not PII.
- **Subjects** hold portable attestations tied to their Stellar address.
- **Verifiers** (including *other contracts*) check attestations with one
  call: `verify(subject, claim_type)`.
- **Regulators / escrows / gates** rely on *contract-to-contract* verification
  without trusting a web API.

## Core concepts

### Commitment, not PII

Issuers never send raw claim data to the contract. They compute

```
claim_hash = sha256(claim_value ‖ salt)
```

and store only `claim_hash`. To prove a claim later, the subject presents
`(claim_value, salt)` and the contract recomputes the digest and compares —
the raw value never touches the ledger and is never revealed to anyone who
doesn't already know it.

### Identity binding

The canonical subject identity is the Soroban [`Address`] — the same address
that signs transactions. Attestations are keyed by the exact
`(subject, claim_type)` pair, so a verification for address **A** can never be
satisfied by an attestation issued to address **B** (no subject substitution).

### Issuer registry & authorization

An `admin` (set once in the contract constructor) manages the issuer registry.
Every issuance requires the *issuer's* signature via `Address::require_auth` —
an attacker cannot forge an attestation without an authorized issuer's key, and
transactions carry sequence numbers so signed operations cannot be replayed.

### Persistent revocation

Revocation writes `revoked: true` to persistent storage and emits an
`attestation_revoked` event. A revoked attestation fails **every** verification
path, including cross-contract calls. Revoked records are retained (not
deleted) so the history is auditable.

## Contract interface

| Function | Signature | Notes |
|---|---|---|
| `__constructor` | `(admin: Address)` | Admin is bound atomically at deploy; no front-running window |
| `add_issuer` | `(issuer: Address) -> Result` | Admin only |
| `remove_issuer` | `(issuer: Address) -> Result` | Admin only; existing attestations unaffected until revoked/expired |
| `is_issuer` | `(address: Address) -> bool` | Public read |
| `issue_attestation` | `(issuer, subject, claim_type: Symbol, claim_hash: BytesN<32>, expiry: u64) -> Result<u32>` | Issuer only; returns attestation id |
| `revoke` | `(caller, attestation_id: u32) -> Result` | Issuing issuer or admin only |
| `verify` | `(subject, claim_type: Symbol) -> bool` | Total function; never panics; cross-contract safe |
| `get_attestation` | `(attestation_id: u32) -> Result<Attestation>` | Full record (commitment only) |
| `verify_claim_commitment` | `(subject, claim_type, claim_value: Bytes, salt: Bytes) -> bool` | Selective disclosure |

### Errors

`Unauthorized`, `NotFound`, `Expired`, `Revoked`, `InvalidExpiry`,
`InvalidSubject`, `InvalidIssuer`, `InvalidClaim`, `AlreadyIssued`,
`NotInitialized`, `AlreadyInitialized` — stable, documented codes that the
[backend SDK](../Attestation-backend-sdk) and frontend map to typed errors.

### Events (indexed by the backend)

| Event | Topic(s) | Data |
|---|---|---|
| `contract_initialized` | — | `admin` |
| `issuer_added` / `issuer_removed` | — | `issuer` |
| `attestation_issued` | `id` | `subject`, `claim_type`, `issuer`, `expiry` |
| `attestation_revoked` | `id` | `revoker` |

Event data is a self-describing map keyed by field name, so off-chain indexers
decode it without external ABI files.

## Repository layout

```
.
├── src/
│   ├── lib.rs        # Entrypoints + authorization + orchestration
│   ├── types.rs      # Attestation, DataKey, error codes, events
│   ├── storage.rs    # TTL management helpers
│   └── test.rs       # 28 unit tests
├── tests/
│   └── integration.rs# Cross-contract consumer tests (3)
├── scripts/          # build / optimize / deploy-testnet / invoke-testnet
├── deployments/      # testnet.json — pinned deployed addresses
├── Makefile          # Developer workflow
└── .github/workflows/ci.yml
```

## Getting started

```bash
# 1. Toolchain (rust-toolchain.toml pins the exact version + wasm32v1-none)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Build + test
cargo build
cargo test          # 31 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# 3. Wasm + optimize (requires the stellar CLI: https://github.com/stellar/stellar-cli)
make wasm
make optimize       # -> target/optimized/attestation_contract.wasm (~20 KB)
```

## Build, test, deploy

```bash
# Deploy to testnet (creates a funded keypair + deploys + writes deployments/testnet.json)
stellar keys generate --fund myissuer
STELLAR_SOURCE_ACCOUNT=myissuer make deploy-testnet

# Invoke on testnet (arg values are bare — see the CLI's implicit --help)
stellar contract invoke --network testnet --source myissuer \
  --id <CONTRACT_ID> -- add_issuer --issuer G...
stellar contract invoke --network testnet --source myissuer \
  --id <CONTRACT_ID> -- issue_attestation --issuer G... --subject G... \
  --claim_type kyc_verified --claim_hash <64-hex> --expiry <unix-secs>
stellar contract invoke --network testnet --source myissuer \
  --id <CONTRACT_ID> -- verify --subject G... --claim_type kyc_verified
```

Full walkthrough: [`DEPLOYMENT.md`](DEPLOYMENT.md).

## Deployed testnet addresses

| Component | Address |
|---|---|
| Attestation contract | `CB2MGYTG6MIIDYWVB5BV4FLEF7KRDEF556JMVZXSZ22XALB7MUC7LU2S` |

The single source of truth is [`deployments/testnet.json`](deployments/testnet.json) —
consumed by the backend SDK and frontend.

## Documentation

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — storage model, TTL strategy, event flow
- [`SECURITY.md`](SECURITY.md) — threat model and mitigations
- [`DEPLOYMENT.md`](DEPLOYMENT.md) — end-to-end testnet deployment walkthrough
- [`CONTRACT.md`](CONTRACT.md) — protocol spec: functions, errors, events, commitment scheme

## Security

The full threat model lives in [`SECURITY.md`](SECURITY.md). Highlights:

- **Unauthorized issuer** → `require_auth` + registry check on every issue.
- **Forged attestation** → impossible without an issuer's signature.
- **Replay** → transaction sequence numbers + `(subject, claim_type)` uniqueness.
- **Subject substitution** → index keyed by the exact `(subject, claim_type)` pair.
- **Issuer compromise** → admin can remove the issuer; attestations can be revoked.
- **Revocation bypass** → revocation is persistent and checked on every path.
- **Expiry** → `expiry` is enforced at read time, on-chain.

Report vulnerabilities privately — see [`SECURITY.md`](SECURITY.md).

---

**This repository is part of the reusable Stellar attestation stack:**
[`Attestation-backend-sdk`](https://github.com/olaleyeolajide81-sketch/Attestation-backend-sdk)
(TypeScript/Python SDKs, caching, API, indexer) and
[`Attestation-frontend`](https://github.com/olaleyeolajide81-sketch/Attestation-frontend)
(React dApp with wallet auth and a contract-to-contract USDC escrow demo).