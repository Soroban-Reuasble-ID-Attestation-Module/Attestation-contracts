//! End-to-end integration test for the attestation-gated escrow.
//!
//! Exercises the complete flow through the public interfaces of both
//! contracts: register issuer → issue attestation → deposit a SAC token
//! into the escrow → release gated by the on-chain `verify()` call →
//! beneficiary receives funds. Also covers the blocked path (no valid
//! attestation) and the depositor clawback.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Bytes, BytesN, Env, Symbol,
};

use attestation_contract::{
    AttestationContract, AttestationContractArgs, AttestationContractClient,
};
use attestation_escrow_contract::{
    AttestationEscrow, AttestationEscrowArgs, AttestationEscrowClient, EscrowError,
};

const NOW: u64 = 1_700_000_000;
const HOUR: u64 = 3_600;
const YEAR: u64 = 365 * 24 * HOUR;

fn commit(env: &Env, value: &str, salt: &str) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&Bytes::from_slice(env, value.as_bytes()));
    preimage.append(&Bytes::from_slice(env, salt.as_bytes()));
    env.crypto().sha256(&preimage).into()
}

struct Deployed {
    env: Env,
    asset: Address,
    attestation: Address,
    escrow: Address,
    issuer: Address,
    subject: Address,
    beneficiary: Address,
    claim_type: Symbol,
}

fn deploy() -> Deployed {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let issuer = Address::generate(&env);
    let subject = Address::generate(&env);
    let beneficiary = Address::generate(&env);
    let claim_type = Symbol::new(&env, "kyc_verified");

    let asset = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    token::StellarAssetClient::new(&env, &asset).mint(&subject, &10_000_000);

    let attestation = env.register(
        AttestationContract,
        AttestationContractArgs::__constructor(&admin),
    );
    let attestation_client = AttestationContractClient::new(&env, &attestation);
    attestation_client.add_issuer(&issuer);

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

    Deployed {
        env,
        asset,
        attestation,
        escrow,
        issuer,
        subject,
        beneficiary,
        claim_type,
    }
}

fn escrow(d: &Deployed) -> AttestationEscrowClient<'_> {
    AttestationEscrowClient::new(&d.env, &d.escrow)
}

fn attestation(d: &Deployed) -> AttestationContractClient<'_> {
    AttestationContractClient::new(&d.env, &d.attestation)
}

fn balance(d: &Deployed, address: &Address) -> i128 {
    token::Client::new(&d.env, &d.asset).balance(address)
}

#[test]
fn full_flow_issue_deposit_release() {
    let d = deploy();
    let escrow = escrow(&d);

    // 1. Issuer issues a kyc_verified attestation for the subject.
    attestation(&d).issue_attestation(
        &d.issuer,
        &d.subject,
        &d.claim_type,
        &commit(&d.env, "passport:AB123", "s3cret"),
        &(NOW + YEAR),
    );

    // 2. The subject deposits USDC-equivalent into the escrow.
    escrow.deposit(&d.subject, &1_000_000);
    assert_eq!(escrow.get_balance(), 1_000_000);

    // 3. Release is gated on-chain by the attestation contract's verify().
    escrow.release(&1_000_000);
    assert_eq!(balance(&d, &d.beneficiary), 1_000_000);
    assert_eq!(escrow.get_balance(), 0);
    assert!(escrow.is_released());

    // 4. The escrow is closed after release.
    let err = escrow.try_release(&1).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::EscrowClosed);
}

#[test]
fn full_flow_release_blocked_and_clawback() {
    let d = deploy();
    let escrow = escrow(&d);

    // No attestation exists: deposit succeeds, release must be refused.
    escrow.deposit(&d.subject, &1_000_000);
    let err = escrow.try_release(&1_000_000).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::AttestationNotVerified);
    assert_eq!(balance(&d, &d.beneficiary), 0);

    // The subject can claw the funds back before release.
    escrow.withdraw(&d.subject, &1_000_000);
    assert_eq!(escrow.get_balance(), 0);
    assert_eq!(balance(&d, &d.subject), 10_000_000);
}

#[test]
fn full_flow_revocation_blocks_release() {
    let d = deploy();
    let escrow = escrow(&d);

    let id = attestation(&d).issue_attestation(
        &d.issuer,
        &d.subject,
        &d.claim_type,
        &commit(&d.env, "passport:AB123", "s3cret"),
        &(NOW + YEAR),
    );
    escrow.deposit(&d.subject, &500_000);

    // Issuer revokes: release must now be refused on-chain.
    attestation(&d).revoke(&d.issuer, &id);
    let err = escrow.try_release(&500_000).unwrap_err().unwrap();
    assert_eq!(err, EscrowError::AttestationNotVerified);
    assert_eq!(escrow.get_balance(), 500_000);
}
