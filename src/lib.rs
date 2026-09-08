#![no_std]

//! # Attestation Contract
//!
//! A reusable Stellar identity & attestation protocol.
//!
//! Authorized issuers issue cryptographic attestations bound to Soroban
//! [`Address`] subjects. Attestations store only cryptographic commitments
//! (SHA-256 digests) of claim values — never raw KYC/PII. Issuance,
//! revocation, and verification are enforced on-chain, with persistent
//! revocation state and full selective-disclosure support.
//!
//! ## Threat model
//!
//! - **Unauthorized issuer**: only addresses registered by the admin may
//!   issue; `Address::require_auth` binds every issuance to a signed
//!   transaction.
//! - **Forged attestation**: attestations are keyed by `(subject,
//!   claim_type)` with a unique id; a forged record would require an
//!   authorized issuer's signature.
//! - **Replay**: every mutating call requires authentication and every
//!   transaction carries a sequence number; duplicates are additionally
//!   rejected by the `(subject, claim_type)` uniqueness index.
//! - **Subject substitution**: verification resolves the exact
//!   `(subject, claim_type)` pair; a different subject can never satisfy a
//!   verification for another address.
//! - **Revocation bypass**: revocation is persisted on-chain and checked on
//!   every verification path, including cross-contract calls.
//!
//! See `SECURITY.md` for the full analysis.

mod storage;
#[cfg(test)]
mod test;
mod types;

use soroban_sdk::{contract, contractimpl, Address, Env};

use crate::storage::{extend_instance_ttl, extend_persistent_ttl};
use crate::types::{
    Attestation, AttestationError, AttestationIssued, AttestationRevoked, ContractInitialized,
    DataKey, IssuerAdded, IssuerRemoved,
};

#[contract]
pub struct AttestationContract;

#[contractimpl]
impl AttestationContract {
    /// Constructor: binds the `admin` account that manages the issuer
    /// registry and initializes the id counter.
    ///
    /// Running initialization in the constructor (rather than a separate
    /// `initialize` entrypoint) means the admin is set atomically at deploy
    /// time — there is no window in which a third party could front-run an
    /// `initialize` call and seize control of the registry.
    pub fn __constructor(env: Env, admin: Address) {
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::NextId, &1u32);
        extend_instance_ttl(&env);

        env.events().publish_event(&ContractInitialized {
            admin: admin.clone(),
        });
    }

    /// Register `issuer` as an authorized attestation issuer. Admin only.
    pub fn add_issuer(env: Env, issuer: Address) -> Result<(), AttestationError> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let key = DataKey::Issuer(issuer.clone());
        if env.storage().persistent().has(&key) {
            return Err(AttestationError::InvalidIssuer);
        }
        env.storage().persistent().set(&key, &());
        extend_persistent_ttl(&env, &key);
        extend_instance_ttl(&env);

        env.events().publish_event(&IssuerAdded { issuer });
        Ok(())
    }

    /// Remove `issuer` from the registry. Admin only.
    ///
    /// Existing attestations remain valid until revoked or expired; the
    /// removed issuer can no longer issue new ones.
    pub fn remove_issuer(env: Env, issuer: Address) -> Result<(), AttestationError> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let key = DataKey::Issuer(issuer.clone());
        if !env.storage().persistent().has(&key) {
            return Err(AttestationError::InvalidIssuer);
        }
        env.storage().persistent().remove(&key);

        env.events().publish_event(&IssuerRemoved { issuer });
        Ok(())
    }

    /// Returns `true` if `address` is a registered issuer.
    pub fn is_issuer(env: Env, address: Address) -> bool {
        if !env.storage().instance().has(&DataKey::Admin) {
            return false;
        }
        env.storage().persistent().has(&DataKey::Issuer(address))
    }

    /// Issue a new attestation for `subject`.
    ///
    /// Only registered issuers may call this. `claim_hash` must be the
    /// SHA-256 commitment of the claim value (with salt); the raw claim is
    /// never stored or transmitted to the contract. `expiry` is a Unix
    /// timestamp (seconds) and must be strictly in the future.
    ///
    /// At most one *active* attestation may exist per `(subject,
    /// claim_type)`. If a previous attestation was revoked, a fresh
    /// attestation may be issued; the revoked record is retained for
    /// auditability.
    pub fn issue_attestation(
        env: Env,
        issuer: Address,
        subject: Address,
        claim_type: soroban_sdk::Symbol,
        claim_hash: soroban_sdk::BytesN<32>,
        expiry: u64,
    ) -> Result<u32, AttestationError> {
        Self::require_initialized(&env)?;
        issuer.require_auth();

        if !env
            .storage()
            .persistent()
            .has(&DataKey::Issuer(issuer.clone()))
        {
            return Err(AttestationError::Unauthorized);
        }
        // Reject an all-zero commitment: it can never be the SHA-256 of a
        // real claim preimage, so accepting it would allow a meaningless
        // attestation that can never be selectively disclosed.
        if claim_hash == soroban_sdk::BytesN::from_array(&env, &[0u8; 32]) {
            return Err(AttestationError::InvalidClaim);
        }

        let now = env.ledger().timestamp();
        if expiry <= now {
            return Err(AttestationError::InvalidExpiry);
        }

        // Enforce one active attestation per (subject, claim_type).
        let index_key = DataKey::SubjectIndex(subject.clone(), claim_type.clone());
        if let Some(existing_id) = env.storage().persistent().get::<DataKey, u32>(&index_key) {
            let existing: Attestation = env
                .storage()
                .persistent()
                .get(&DataKey::Attestation(existing_id))
                .ok_or(AttestationError::NotFound)?;
            if !existing.revoked {
                return Err(AttestationError::AlreadyIssued);
            }
            extend_persistent_ttl(&env, &index_key);
        }

        let id: u32 = env.storage().instance().get(&DataKey::NextId).unwrap_or(1);
        let attestation = Attestation {
            id,
            subject: subject.clone(),
            claim_type: claim_type.clone(),
            claim_hash,
            issuer: issuer.clone(),
            issued_at: now,
            expiry,
            revoked: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Attestation(id), &attestation);
        env.storage().persistent().set(&index_key, &id);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        extend_persistent_ttl(&env, &DataKey::Attestation(id));
        extend_persistent_ttl(&env, &index_key);
        extend_instance_ttl(&env);

        env.events().publish_event(&AttestationIssued {
            id,
            subject: subject.clone(),
            claim_type: claim_type.clone(),
            issuer: issuer.clone(),
            expiry,
        });
        Ok(id)
    }

    /// Revoke an attestation by id. The issuing issuer or the admin may
    /// revoke. Revocation is persistent and irreversible: a revoked
    /// attestation fails every verification path.
    pub fn revoke(env: Env, caller: Address, attestation_id: u32) -> Result<(), AttestationError> {
        Self::require_initialized(&env)?;
        caller.require_auth();

        let key = DataKey::Attestation(attestation_id);
        let attestation: Attestation = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(AttestationError::NotFound)?;

        let admin = Self::admin(&env)?;
        if caller != attestation.issuer && caller != admin {
            return Err(AttestationError::Unauthorized);
        }
        if attestation.revoked {
            return Err(AttestationError::Revoked);
        }

        let mut updated = attestation.clone();
        updated.revoked = true;
        env.storage().persistent().set(&key, &updated);
        extend_persistent_ttl(&env, &key);
        extend_instance_ttl(&env);

        env.events().publish_event(&AttestationRevoked {
            id: attestation_id,
            revoker: caller,
        });
        Ok(())
    }

    /// Verify that `subject` holds an active attestation for `claim_type`.
    ///
    /// Returns `false` when no attestation exists for the pair, when it has
    /// been revoked, or when it has expired. This is the entrypoint other
    /// contracts (e.g. an escrow) call cross-contract; it never panics and
    /// never requires authorization.
    pub fn verify(env: Env, subject: Address, claim_type: soroban_sdk::Symbol) -> bool {
        if !env.storage().instance().has(&DataKey::Admin) {
            return false;
        }
        let index_key = DataKey::SubjectIndex(subject, claim_type);
        let Some(id) = env.storage().persistent().get::<DataKey, u32>(&index_key) else {
            return false;
        };
        extend_persistent_ttl(&env, &index_key);

        let key = DataKey::Attestation(id);
        let Some(attestation) = env.storage().persistent().get::<DataKey, Attestation>(&key) else {
            return false;
        };
        extend_persistent_ttl(&env, &key);

        if attestation.revoked {
            return false;
        }
        let now = env.ledger().timestamp();
        attestation.expiry > now
    }

    /// Fetch the full attestation record by id. The record contains only
    /// the cryptographic commitment — never raw claim data.
    pub fn get_attestation(env: Env, attestation_id: u32) -> Result<Attestation, AttestationError> {
        let key = DataKey::Attestation(attestation_id);
        let attestation = env
            .storage()
            .persistent()
            .get::<DataKey, Attestation>(&key)
            .ok_or(AttestationError::NotFound)?;
        extend_persistent_ttl(&env, &key);
        extend_instance_ttl(&env);
        Ok(attestation)
    }

    /// Selective disclosure: prove that `claim_value` (salted with `salt`)
    /// matches the commitment stored for `subject`'s `claim_type`.
    ///
    /// The contract recomputes `sha256(claim_value || salt)` and compares it
    /// to the stored commitment, so the raw claim value never touches the
    /// ledger — only its digest is checked. Fails (returns `false`) for
    /// missing, revoked, or expired attestations.
    ///
    /// The commitment scheme is `sha256(claim_value || salt)` and must be
    /// reproduced identically by off-chain SDKs (see the backend SDK docs).
    pub fn verify_claim_commitment(
        env: Env,
        subject: Address,
        claim_type: soroban_sdk::Symbol,
        claim_value: soroban_sdk::Bytes,
        salt: soroban_sdk::Bytes,
    ) -> bool {
        if !env.storage().instance().has(&DataKey::Admin) {
            return false;
        }
        let index_key = DataKey::SubjectIndex(subject, claim_type);
        let Some(id) = env.storage().persistent().get::<DataKey, u32>(&index_key) else {
            return false;
        };
        extend_persistent_ttl(&env, &index_key);

        let key = DataKey::Attestation(id);
        let Some(attestation) = env.storage().persistent().get::<DataKey, Attestation>(&key) else {
            return false;
        };
        extend_persistent_ttl(&env, &key);

        if attestation.revoked || attestation.expiry <= env.ledger().timestamp() {
            return false;
        }

        let mut preimage = soroban_sdk::Bytes::new(&env);
        preimage.append(&claim_value);
        preimage.append(&salt);
        let digest: soroban_sdk::BytesN<32> = env.crypto().sha256(&preimage).into();
        digest == attestation.claim_hash
    }
}

impl AttestationContract {
    /// Reads the admin address, failing with `NotInitialized` when the
    /// contract has not been initialized yet.
    fn admin(env: &Env) -> Result<Address, AttestationError> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(AttestationError::NotInitialized)
    }

    /// Guards entrypoints that require the contract to be initialized.
    fn require_initialized(env: &Env) -> Result<(), AttestationError> {
        if env.storage().instance().has(&DataKey::Admin) {
            Ok(())
        } else {
            Err(AttestationError::NotInitialized)
        }
    }
}
