//! Cross-contract integration test.
//!
//! The protocol's core contract-to-contract story: a *consumer* contract
//! (in this test, a stand-in for an escrow or gating contract) calls
//! `verify()` on the attestation contract and gates its behavior on the
//! result. This proves the `verify()` entrypoint is callable cross-contract
//! and that revocation/expiry state is honored through that path — not just
//! in direct client calls.

use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, testutils::Ledger, Address, Bytes, BytesN,
    Env, Symbol,
};

use attestation_contract::{
    AttestationContract, AttestationContractArgs, AttestationContractClient,
};

const NOW: u64 = 1_700_000_000;
const HOUR: u64 = 3_600;

/// Minimal stand-in for a downstream contract (escrow, gating, payroll)
/// that relies on the attestation contract's `verify()`.
#[contract]
pub struct ConsumerContract;

#[contractimpl]
impl ConsumerContract {
    /// Returns true only if the attestation contract confirms `subject`
    /// holds an active `claim_type` attestation.
    pub fn check(
        env: Env,
        attestation_contract: Address,
        subject: Address,
        claim_type: Symbol,
    ) -> bool {
        AttestationContractClient::new(&env, &attestation_contract).verify(&subject, &claim_type)
    }
}

fn commit(env: &Env, value: &str, salt: &str) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&Bytes::from_slice(env, value.as_bytes()));
    preimage.append(&Bytes::from_slice(env, salt.as_bytes()));
    env.crypto().sha256(&preimage).into()
}

/// Full happy path through a consumer contract: register issuer, issue,
/// consumer verifies, revoke, consumer verification fails.
#[test]
fn consumer_gate_honors_attestation_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let issuer = Address::generate(&env);
    let subject = Address::generate(&env);
    let claim_type = Symbol::new(&env, "kyc_verified");

    let attestation_id =
        env.register(AttestationContract, AttestationContractArgs::__constructor(&admin));
    let attestation = AttestationContractClient::new(&env, &attestation_id);

    attestation.add_issuer(&issuer);
    let id = attestation.issue_attestation(
        &issuer,
        &subject,
        &claim_type,
        &commit(&env, "passport:AB123", "s3cret"),
        &(NOW + 365 * 24 * HOUR),
    );

    let consumer_id = env.register(ConsumerContract, ());
    let consumer = ConsumerContractClient::new(&env, &consumer_id);

    // The consumer sees the active attestation...
    assert!(consumer.check(&attestation_id, &subject, &claim_type));
    // ...and never sees one for a different subject (no substitution).
    assert!(!consumer.check(&attestation_id, &Address::generate(&env), &claim_type));
    // ...nor a different claim type.
    assert!(!consumer.check(&attestation_id, &subject, &Symbol::new(&env, "accredited_investor")));

    // Revocation must propagate through the cross-contract path.
    attestation.revoke(&issuer, &id);
    assert!(!consumer.check(&attestation_id, &subject, &claim_type));
}

/// An uninitialized (never-issued) pair must fail verification through the
/// consumer too.
#[test]
fn consumer_sees_no_attestation_before_issuance() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let subject = Address::generate(&env);

    let attestation_id =
        env.register(AttestationContract, AttestationContractArgs::__constructor(&admin));
    let consumer_id = env.register(ConsumerContract, ());
    let consumer = ConsumerContractClient::new(&env, &consumer_id);

    assert!(!consumer.check(&attestation_id, &subject, &Symbol::new(&env, "kyc_verified")));
}

/// Expiry must propagate through the cross-contract path as well.
#[test]
fn consumer_sees_expiry() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let issuer = Address::generate(&env);
    let subject = Address::generate(&env);
    let claim_type = Symbol::new(&env, "kyc_verified");

    let attestation_id =
        env.register(AttestationContract, AttestationContractArgs::__constructor(&admin));
    let attestation = AttestationContractClient::new(&env, &attestation_id);
    attestation.add_issuer(&issuer);
    attestation.issue_attestation(
        &issuer,
        &subject,
        &claim_type,
        &commit(&env, "v", "s"),
        &(NOW + HOUR),
    );

    let consumer_id = env.register(ConsumerContract, ());
    let consumer = ConsumerContractClient::new(&env, &consumer_id);
    assert!(consumer.check(&attestation_id, &subject, &claim_type));

    env.ledger().set_timestamp(NOW + HOUR + 1);
    assert!(!consumer.check(&attestation_id, &subject, &claim_type));
}