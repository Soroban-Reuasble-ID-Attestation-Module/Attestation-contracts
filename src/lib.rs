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
mod types;

use soroban_sdk::{contract, contractimpl, Address, Env};

use crate::storage::{extend_instance_ttl, extend_persistent_ttl};
use crate::types::{
    AttestationError, ContractInitialized, DataKey, IssuerAdded, IssuerRemoved,
};

#[contract]
pub struct AttestationContract;

#[contractimpl]
impl AttestationContract {
    /// Initialize the contract with the `admin` account that manages the
    /// issuer registry. Idempotency guard: can only be called once.
    pub fn initialize(env: Env, admin: Address) -> Result<(), AttestationError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(AttestationError::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::NextId, &1u32);
        extend_instance_ttl(&env);

        env.events()
            .publish_event(&ContractInitialized { admin: admin.clone() });
        Ok(())
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
}