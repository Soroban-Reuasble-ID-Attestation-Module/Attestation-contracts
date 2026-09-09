# Changelog

All notable changes to this repository are documented here, following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `v0.1` contract interface stabilized (see `CONTRACT.md`):
  - `__constructor(admin)`, `add_issuer`, `remove_issuer`, `is_issuer`
  - `issue_attestation`, `revoke`, `verify`, `get_attestation`,
    `verify_claim_commitment`
  - Typed events: `contract_initialized`, `issuer_added`, `issuer_removed`,
    `attestation_issued`, `attestation_revoked`
  - Stable error codes 1–11
- 28 unit tests and 3 cross-contract integration tests (31 total).
- Tooling: Makefile, build/optimize/deploy/invoke scripts, GitHub Actions CI.
- Live testnet deployment pinned in `deployments/testnet.json`
  (`CB2MGYTG6MIIDYWVB5BV4FLEF7KRDEF556JMVZXSZ22XALB7MUC7LU2S`) with the full
  lifecycle verified on-chain.
- **New: attestation-gated escrow contract** (`escrow-contract/`):
  - `__constructor(admin, asset, attestation_contract, subject, claim_type,
    beneficiary)`, `deposit`, `release`, `withdraw`, and read helpers
    (`get_balance`, `get_deposit`, `is_released`, `config`).
  - The release gate is decided **on-chain** via a cross-contract `verify()`
    call through a local `#[contractclient]` interface (the attestation wasm
    is never linked into the escrow).
  - Per-depositor balances with clawback before release; no privileged
    movement functions; escrow closes after the first release.
  - Stable escrow error codes 1–6 and events `deposited` / `released` /
    `withdrawn`.
  - 14 new tests (11 unit + 3 full-flow integration), 45 total; CI covers
    the workspace.
  - `make deploy-escrow-testnet` / `scripts/deploy-escrow-testnet.sh`
    append the deployed escrow id to `deployments/testnet.json`; docs updated
    (README, ARCHITECTURE §9, SECURITY §2.11–2.13, DEPLOYMENT §4b,
    CONTRACT §4b).

## [0.1.0] - 2026-09-08

### Added

- Soroban contract crate (soroban-sdk 27, `wasm32v1-none`).
- Issuer registry with constructor-bound admin (no front-running window).
- Cryptographic attestation issuance bound to `Address` subjects.
- Persistent, irreversible revocation.
- On-chain `verify()` designed as a total, cross-contract-safe function.
- Selective disclosure via `verify_claim_commitment`.
- TTL management (extend-on-touch, 31-day instance / 365-day persistent).
- Documentation: README, ARCHITECTURE, SECURITY, DEPLOYMENT, CONTRACT.