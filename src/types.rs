//! On-chain data types, storage keys, error codes, and contract events.

use soroban_sdk::{contracterror, contractevent, contracttype, Address, BytesN, Symbol};

/// Storage keys used across instance and persistent storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance storage: the admin account that manages the issuer registry.
    Admin,
    /// Instance storage: monotonic counter for attestation ids.
    NextId,
    /// Persistent storage: presence marker for a registered issuer.
    Issuer(Address),
    /// Persistent storage: attestation record keyed by id.
    Attestation(u32),
    /// Persistent storage: resolves (subject, claim_type) to an attestation id.
    SubjectIndex(Address, Symbol),
}

/// A cryptographic attestation bound to a Stellar address subject.
///
/// Only the cryptographic commitment (`claim_hash`) is stored on-chain.
/// Raw claim values and PII are never persisted.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attestation {
    /// Stable, monotonically increasing identifier of this attestation.
    pub id: u32,
    /// The Stellar address the attestation is bound to.
    pub subject: Address,
    /// Claim type, e.g. `kyc_verified`, `accredited_investor`.
    pub claim_type: Symbol,
    /// SHA-256 commitment of the claim value (with salt).
    pub claim_hash: BytesN<32>,
    /// The registered issuer that issued this attestation.
    pub issuer: Address,
    /// Ledger timestamp (Unix seconds) of issuance.
    pub issued_at: u64,
    /// Unix seconds at which the attestation becomes invalid.
    pub expiry: u64,
    /// Persistent revocation flag; revoked attestations always fail verification.
    pub revoked: bool,
}

/// Contract error codes. These values are part of the public interface and
/// are depended on by SDKs, indexers, and off-chain clients — keep stable.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AttestationError {
    /// Caller is not authorized for this operation.
    Unauthorized = 1,
    /// Attestation (or subject/claim-type pair) does not exist.
    NotFound = 2,
    /// Attestation exists but is expired.
    Expired = 3,
    /// Attestation exists but has been revoked.
    Revoked = 4,
    /// Expiry timestamp is not strictly in the future.
    InvalidExpiry = 5,
    /// The supplied subject is not a valid identity.
    InvalidSubject = 6,
    /// The address is not a registered issuer.
    InvalidIssuer = 7,
    /// The supplied claim commitment is invalid.
    InvalidClaim = 8,
    /// An active attestation already exists for (subject, claim_type).
    AlreadyIssued = 9,
    /// The contract has not been initialized.
    NotInitialized = 10,
    /// The contract has already been initialized.
    AlreadyInitialized = 11,
}

// ---------------------------------------------------------------------------
// Contract events
// ---------------------------------------------------------------------------
// Events are indexed by the backend event indexer (see
// Attestation-backend-sdk). Field names are encoded in the event data so
// consumers can decode without external ABI knowledge.

/// Emitted once on `initialize`.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractInitialized {
    pub admin: Address,
}

/// Emitted when an issuer is registered.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuerAdded {
    pub issuer: Address,
}

/// Emitted when an issuer is removed from the registry.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuerRemoved {
    pub issuer: Address,
}

/// Emitted when an attestation is issued.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationIssued {
    /// Attestation id — also published as a topic so indexers can filter.
    #[topic]
    pub id: u32,
    pub subject: Address,
    pub claim_type: Symbol,
    pub issuer: Address,
    pub expiry: u64,
}

/// Emitted when an attestation is revoked.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestationRevoked {
    /// Attestation id — also published as a topic so indexers can filter.
    #[topic]
    pub id: u32,
    pub revoker: Address,
}
