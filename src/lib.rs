#![no_std]

//! # Attestation Contract
//!
//! A reusable Stellar identity & attestation protocol.
//!
//! Authorized issuers issue cryptographic attestations bound to Soroban
//! [`Address`] subjects. Attestations store only cryptographic commitments
//! (SHA-256 digests) of claim values — never raw KYC/PII. Issuance,
//! revocation and verification are enforced on-chain, with persistent
//! revocation state and full selective-disclosure support.

use soroban_sdk::{contract, contractimpl};

#[contract]
pub struct AttestationContract;

#[contractimpl]
impl AttestationContract {
    /// Placeholder; real entrypoints are added as the contract is built out.
    pub fn hello(_env: soroban_sdk::Env) -> soroban_sdk::Symbol {
        soroban_sdk::Symbol::new(&soroban_sdk::Env::default(), "attestation")
    }
}