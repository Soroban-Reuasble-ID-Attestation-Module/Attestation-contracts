# Security

Security model, threat analysis, and operational guidance for the attestation
contract. **The contract stores only cryptographic commitments — never raw
KYC/PII.** This is enforced at the type level (`claim_hash: BytesN<32>`) and
by the fact that no entrypoint accepts raw claim data.

## 1. Trust model

Three roles exist:

| Role | Capabilities | Trust assumption |
|---|---|---|
| **Admin** | Manages the issuer registry; may revoke any attestation | The deployer, chosen once in the constructor. Single point of compromise → see §6.2. |
| **Issuer** | Issues attestations for its registered address; revokes its own | Registered by the admin. Can create *false* attestations for its own subjects — that is the point of issuance, not a vulnerability. |
| **Anyone** | Verifies, reads, selectively discloses | Untrusted. All reads are total functions with no side effects. |

## 2. Threats and mitigations

### 2.1 Unauthorized issuer (fake attestations)

- **Threat:** An unregistered address issues an attestation.
- **Mitigation:** `issue_attestation` calls `issuer.require_auth()` (the caller
  must be the issuer and must sign the transaction) **and** checks the
  persistent issuer registry. Both must pass; the contract body never runs
  otherwise. Test: `non_issuer_cannot_issue`.

### 2.2 Forged attestation

- **Threat:** An attacker fabricates a valid-looking record.
- **Mitigation:** Records are only written inside `issue_attestation` under the
  caller's auth. An attacker cannot inject a record directly — Soroban storage
  is only writable through contract code, and all writes require the issuer's
  signature.

### 2.3 Replay

- **Threat:** A captured issuance/revocation transaction is replayed.
- **Mitigation:** Stellar transactions carry account sequence numbers (a
  replayed transaction has an invalid sequence and is rejected), and the
  `(subject, claim_type)` uniqueness index makes duplicate issuance fail with
  `AlreadyIssued`. Revoking an already-revoked attestation fails with
  `Revoked`.

### 2.4 Subject substitution

- **Threat:** An attacker uses subject **B**'s attestation to pass a
  verification for subject **A**.
- **Mitigation:** The index key is the exact `(subject, claim_type)` pair and
  verification resolves through that key only. An attestation for B can never
  be found under A's key. Tests: `verify_fails_on_subject_substitution` and
  the cross-contract consumer test.

### 2.5 Issuer compromise

- **Threat:** An issuer's key is stolen.
- **Mitigation:** The admin can `remove_issuer` (stops new issuance
  immediately) and revoke any attestation that key produced. Removal takes
  effect at the next issuance attempt; existing attestations remain valid
  until revoked or expired — the admin should revoke them in the same
  incident-response step. See §6.2.

### 2.6 Revocation bypass

- **Threat:** A revoked attestation passes verification.
- **Mitigation:** Revocation writes `revoked: true` to the *persistent*
  record — not an in-memory flag. Every verification path
  (`verify`, `verify_claim_commitment`, and any cross-contract consumer)
  checks the flag before returning true. Tests: `verify_fails_after_revocation`,
  `disclosure_fails_for_revoked_or_expired`, cross-contract revocation test.

### 2.7 Expiry bypass

- **Threat:** An expired attestation passes verification.
- **Mitigation:** `expiry` is enforced at read time against
  `env.ledger().timestamp()`; the ledger timestamp cannot be forged by a
  caller. Both verification entrypoints check `expiry > now`. Tests:
  `verify_fails_after_expiry`, `consumer_sees_expiry`.

### 2.8 Front-running initialization

- **Threat (deploy-then-initialize pattern):** attacker calls `initialize`
  before the deployer does and becomes admin.
- **Mitigation:** Not applicable — admin binding happens in the constructor,
  which runs atomically during deployment. There is no `initialize` entrypoint.

### 2.9 Claim-value guessing (selective disclosure)

- **Threat:** An attacker enumerates candidate `(value, salt)` pairs to match
  a stored commitment.
- **Mitigation:** The salt is secret and high-entropy (16+ random bytes per
  claim, never reused). With a random 128-bit salt, enumeration is
  computationally infeasible. **Issuers must generate strong salts and keep
  them secret** — a low-entropy or reused salt weakens this property.

### 2.10 Data availability / state expiry

- **Threat:** Unread attestations expire from ledger storage.
- **Mitigation:** Every read extends the TTL (see `ARCHITECTURE.md` §3), and
  the backend indexer keeps a persistent off-chain copy, so verification
  remains possible even if a record ages out of the ledger.

### 2.11 Escrow: forged or unauthorized release

- **Threat:** An attacker triggers `release()` while the subject holds no
  valid attestation, or redirects funds to themselves.
- **Mitigation:** `release()` is a single path to move funds out, and it
  performs the gate **on-chain** by calling the attestation contract's
  `verify(subject, claim_type)` cross-contract. A missing, revoked, or
  expired attestation returns `Err(AttestationNotVerified)` and no transfer
  occurs. The beneficiary is fixed at construction, so funds cannot be
  redirected regardless of who triggers the release.

### 2.12 Escrow: theft via withdraw or double-spend

- **Threat:** A depositor withdraws more than they deposited, or funds are
  withdrawn twice.
- **Mitigation:** Withdrawals are capped at the depositor's own recorded
  share in the per-depositor map and are disabled once the escrow is closed
  by the first release. The total `Balance` is decremented on every
  movement, so escrow accounting can never go negative.

### 2.13 Escrow: stuck funds / admin abuse

- **Threat:** The admin drains the escrow, or funds are locked forever.
- **Mitigation:** There are no privileged movement functions — the admin
  cannot withdraw on anyone's behalf. Every depositor can claw back their
  own deposit before release, so funds are never locked in and never
  custodial.

## 3. What this contract is *not* responsible for

- **Issuer diligence.** The contract proves *that* a registered issuer issued
  an attestation for a subject — it does not prove the claim is true. A
  registry of reputable issuers is an off-chain governance concern.
- **Key custody.** Compromised admin or issuer keys are out of scope of the
  contract (see incident response below).
- **Off-chain storage of raw claims.** The contract never sees them.

## 4. Testing posture

- 45 automated tests (28 + 3 attestation, 11 + 3 escrow) exercise the
  real Soroban host, including the full attestation → escrow → release
  flow through both contracts and a real SAC token.
- CI blocks on clippy warnings (`-D warnings`) and runs `cargo fmt --check`.
- The contract was additionally exercised **on a live testnet deployment**:
  issue → verify → selective disclosure → duplicate rejection → revoke →
  verify-false (see `README.md`).

## 5. Operational security

- Store issuer/admin secret keys in a hardware wallet or vault; never in
  application code, config files, or the backend API.
- The backend service **never holds signing keys** — signing happens in the
  SDK layer (LocalSigner from env/secret manager, or a RemoteSigner service).
- Rate-limit and authenticate access to the backend API (JWT + RBAC); the
  contract itself needs no rate limiting.

## 6. Incident response

### 6.1 Suspicious attestation

1. Revoke it on-chain (issuer or admin) — takes effect immediately.
2. Emit an off-chain advisory; consumers see `verify() == false` from that
   moment.

### 6.2 Compromised issuer key

1. `remove_issuer` (admin) — blocks new issuance.
2. Revoke all attestations issued by that address.
3. Rotate: register a fresh issuer address if needed.

### 6.3 Compromised admin key

The admin controls the registry, so treat this as critical:

1. There is **no on-chain admin rotation** in this contract version — redeploy
   the contract with a new admin if the trust root is lost.
2. Coordinate with all consumers (backend indexer, frontend, escrow) to point
   at the new contract id.

## 7. Reporting

Please report suspected vulnerabilities privately to the repository owners with
full reproduction steps. Include the affected contract version and, where
possible, a proposed fix. No public disclosure until a fix or mitigation is
shipped.

## 8. Formal properties checklist

- [x] Only registered issuers can issue (`require_auth` + registry).
- [x] Only the issuing issuer or admin can revoke.
- [x] Revocation persists across ledgers and blocks all verification paths.
- [x] Expiry is enforced at read time against the ledger clock.
- [x] One active attestation per `(subject, claim_type)`.
- [x] No raw claim data is ever accepted or stored.
- [x] `verify()` is total (never panics) — safe for contract-to-contract use.
- [x] Admin binding is atomic at deploy (no front-running).
- [ ] Admin rotation on-chain (future work; documented operational workaround above).