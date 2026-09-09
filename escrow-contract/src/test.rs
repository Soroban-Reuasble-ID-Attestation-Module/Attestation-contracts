//! Unit tests for the attestation-gated escrow contract.
//!
//! Covers: deposit balance tracking, per-depositor isolation, release
//! gated by on-chain attestation verification (valid / missing / revoked /
//! expired), release amount validation, depositor clawback before release,
//! and the closed-after-release lifecycle.
//!
//! Tests use `mock_all_auths` (matching the attestation contract's suite);
//! balance isolation between depositors is still enforced by the escrow's
//! own storage, independent of auth mocking.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env, Symbol,
};

use attestation_contract::{
    AttestationContract, AttestationContractArgs, AttestationContractClient,
};

use crate::{AttestationEscrow, AttestationEscrowArgs, AttestationEscrowClient, EscrowError};

/// Unix timestamp used as the ledger time baseline in tests.
const NOW: u64 = 1_700_000_000;
/// One hour in seconds.
const HOUR: u64 = 3_600;
/// One year in seconds.
const YEAR: u64 = 365 * 24 * HOUR;

/// Compute the protocol commitment: `sha256(claim_value || salt)`.
fn commit(env: &Env, value: &str, salt: &str) -> soroban_sdk::BytesN<32> {
    let mut preimage = soroban_sdk::Bytes::new(env);
    preimage.append(&soroban_sdk::Bytes::from_slice(env, value.as_bytes()));
    preimage.append(&soroban_sdk::Bytes::from_slice(env, salt.as_bytes()));
    env.crypto().sha256(&preimage).into()
}

/// Test fixture: a funded SAC token, an initialized attestation contract
/// with one issuer, and an escrow bound to `(subject, claim_type)` paying
/// `beneficiary`.
struct Fixture {
    env: Env,
    asset: Address,
    attestation: Address,
    escrow: Address,
    admin: Address,
    issuer: Address,
    subject: Address,
    beneficiary: Address,
    claim_type: Symbol,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(NOW);

        let admin = Address::generate(&env);
        let issuer = Address::generate(&env);
        let subject = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let claim_type = Symbol::new(&env, "kyc_verified");

        // SAC-compatible token, funded subject.
        let asset = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        token::StellarAssetClient::new(&env, &asset).mint(&subject, &1_000_000);

        // Attestation contract with one registered issuer.
        let attestation = env.register(
            AttestationContract,
            AttestationContractArgs::__constructor(&admin),
        );
        let attestation_client = AttestationContractClient::new(&env, &attestation);
        attestation_client.add_issuer(&issuer);

        // Escrow bound to the subject/claim pair, paying the beneficiary.
        let escrow = env.register(
            AttestationEscrow,
            AttestationEscrowArgs::__constructor(
                &admin,
                &asset,
                &attestation,
                &subject,
                &claim_type,
                &beneficiary,
            ),
        );

        Self {
            env,
            asset,
            attestation,
            escrow,
            admin,
            issuer,
            subject,
            beneficiary,
            claim_type,
        }
    }

    fn escrow(&self) -> AttestationEscrowClient<'_> {
        AttestationEscrowClient::new(&self.env, &self.escrow)
    }

    fn token_balance(&self, address: &Address) -> i128 {
        token::Client::new(&self.env, &self.asset).balance(address)
    }

    /// Issue a valid attestation for the fixture subject.
    fn issue_valid(&self) -> u32 {
        AttestationContractClient::new(&self.env, &self.attestation).issue_attestation(
            &self.issuer,
            &self.subject,
            &self.claim_type,
            &commit(&self.env, "passport:AB123", "s3cret"),
            &(NOW + YEAR),
        )
    }
}

#[test]
fn deposit_tracks_balances_and_total() {
    let f = Fixture::new();
    let escrow = f.escrow();

    escrow.deposit(&f.subject, &1_000);
    assert_eq!(escrow.get_balance(), 1_000);
    assert_eq!(escrow.get_deposit(&f.subject), 1_000);
    assert_eq!(escrow.get_deposit(&f.beneficiary), 0);

    escrow.deposit(&f.subject, &500);
    assert_eq!(escrow.get_balance(), 1_500);
    assert_eq!(escrow.get_deposit(&f.subject), 1_500);
}

#[test]
fn deposit_rejects_zero_or_negative_amounts() {
    let f = Fixture::new();
    let escrow = f.escrow();

    let err = escrow.try_deposit(&f.subject, &0).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::InvalidAmount);
    let err = escrow.try_deposit(&f.subject, &-100).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::InvalidAmount);
    assert_eq!(escrow.get_balance(), 0);
}

#[test]
fn depositors_balances_are_isolated() {
    let f = Fixture::new();
    let escrow = f.escrow();
    let other = Address::generate(&f.env);

    // Fund a second depositor.
    token::StellarAssetClient::new(&f.env, &f.asset).mint(&other, &1_000_000);

    escrow.deposit(&f.subject, &1_000);
    escrow.deposit(&other, &200);

    assert_eq!(escrow.get_balance(), 1_200);
    assert_eq!(escrow.get_deposit(&f.subject), 1_000);
    assert_eq!(escrow.get_deposit(&other), 200);

    // The other depositor cannot claw back more than their own share.
    let err = escrow.try_withdraw(&other, &300).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::InsufficientBalance);
    escrow.withdraw(&other, &200);
    assert_eq!(escrow.get_deposit(&other), 0);
    assert_eq!(escrow.get_balance(), 1_000);
}

#[test]
fn release_transfers_funds_to_beneficiary_when_attestation_valid() {
    let f = Fixture::new();
    let escrow = f.escrow();

    f.issue_valid();
    escrow.deposit(&f.subject, &500);

    escrow.release(&500);

    assert_eq!(f.token_balance(&f.beneficiary), 500);
    assert_eq!(escrow.get_balance(), 0);
    assert!(escrow.is_released());
}

#[test]
fn release_rejected_when_no_attestation_exists() {
    let f = Fixture::new();
    let escrow = f.escrow();

    escrow.deposit(&f.subject, &500);
    let err = escrow.try_release(&500).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::AttestationNotVerified);
    // Funds stay put.
    assert_eq!(f.token_balance(&f.beneficiary), 0);
    assert_eq!(escrow.get_balance(), 500);
    assert!(!escrow.is_released());
}

#[test]
fn release_rejected_when_attestation_revoked() {
    let f = Fixture::new();
    let escrow = f.escrow();

    let id = f.issue_valid();
    AttestationContractClient::new(&f.env, &f.attestation).revoke(&f.issuer, &id);
    escrow.deposit(&f.subject, &500);

    let err = escrow.try_release(&500).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::AttestationNotVerified);
    assert_eq!(escrow.get_balance(), 500);
}

#[test]
fn release_rejected_when_attestation_expired() {
    let f = Fixture::new();
    let escrow = f.escrow();

    let id = AttestationContractClient::new(&f.env, &f.attestation).issue_attestation(
        &f.issuer,
        &f.subject,
        &f.claim_type,
        &commit(&f.env, "passport:AB123", "s3cret"),
        &(NOW + HOUR),
    );
    assert!(id > 0);
    escrow.deposit(&f.subject, &500);

    // Move the ledger past the attestation expiry.
    f.env.ledger().set_timestamp(NOW + 2 * HOUR);
    let err = escrow.try_release(&500).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::AttestationNotVerified);
    assert_eq!(escrow.get_balance(), 500);
}

#[test]
fn release_validates_amount() {
    let f = Fixture::new();
    let escrow = f.escrow();

    f.issue_valid();
    escrow.deposit(&f.subject, &500);

    let err = escrow.try_release(&0).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::InvalidAmount);
    let err = escrow.try_release(&501).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::InsufficientBalance);
    // Partial release is supported.
    escrow.release(&200);
    assert_eq!(f.token_balance(&f.beneficiary), 200);
    assert_eq!(escrow.get_balance(), 300);
    assert!(escrow.is_released());
}

#[test]
fn withdraw_allows_clawback_before_release() {
    let f = Fixture::new();
    let escrow = f.escrow();

    escrow.deposit(&f.subject, &500);
    escrow.withdraw(&f.subject, &200);

    assert_eq!(escrow.get_deposit(&f.subject), 300);
    assert_eq!(escrow.get_balance(), 300);

    escrow.withdraw(&f.subject, &300);
    assert_eq!(escrow.get_balance(), 0);
    assert_eq!(f.token_balance(&f.subject), 1_000_000);
}

#[test]
fn escrow_closes_after_release() {
    let f = Fixture::new();
    let escrow = f.escrow();

    f.issue_valid();
    escrow.deposit(&f.subject, &500);
    escrow.release(&500);

    // Deposits and withdrawals are refused once released.
    let err = escrow.try_deposit(&f.subject, &100).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::EscrowClosed);
    let err = escrow.try_withdraw(&f.subject, &1).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::EscrowClosed);
    let err = escrow.try_release(&1).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::EscrowClosed);
}

#[test]
fn config_exposes_full_escrow_configuration() {
    let f = Fixture::new();
    let escrow = f.escrow();

    let cfg = escrow.config();
    assert_eq!(cfg.admin, f.admin);
    assert_eq!(cfg.asset, f.asset);
    assert_eq!(cfg.attestation_contract, f.attestation);
    assert_eq!(cfg.subject, f.subject);
    assert_eq!(cfg.claim_type, f.claim_type);
    assert_eq!(cfg.beneficiary, f.beneficiary);
    assert!(!cfg.released);
}
