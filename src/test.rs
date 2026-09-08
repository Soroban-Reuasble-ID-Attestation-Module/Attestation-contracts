//! Unit tests for the attestation contract.
//!
//! Covers: lifecycle guards, issuer registry authorization, issuance
//! validation, verification (existence, revocation, expiry, subject
//! substitution), revocation authorization and idempotency, selective
//! disclosure, duplicate prevention, re-issue after revocation, and event
//! emission.

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    xdr::{ContractEventBody, ScVal},
    Address, Bytes, BytesN, Env, Symbol, TryFromVal, TryIntoVal, Val,
};

use crate::{
    Attestation, AttestationContract, AttestationContractArgs, AttestationContractClient,
    AttestationError,
};

/// Unix timestamp used as the ledger time baseline in tests.
const NOW: u64 = 1_700_000_000;
/// One hour in seconds.
const HOUR: u64 = 3_600;

/// Compute the protocol commitment: `sha256(claim_value || salt)`.
fn commit(env: &Env, value: &str, salt: &str) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&Bytes::from_slice(env, value.as_bytes()));
    preimage.append(&Bytes::from_slice(env, salt.as_bytes()));
    env.crypto().sha256(&preimage).into()
}

/// Standard test fixture: initialized contract with one registered issuer.
struct Fixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    issuer: Address,
    subject: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(NOW);

        let admin = Address::generate(&env);
        let issuer = Address::generate(&env);
        let subject = Address::generate(&env);

        let contract_id =
            env.register(AttestationContract, AttestationContractArgs::__constructor(&admin));
        let client = AttestationContractClient::new(&env, &contract_id);
        client.add_issuer(&issuer);

        Self {
            env,
            contract_id,
            admin,
            issuer,
            subject,
        }
    }

    fn client(&self) -> AttestationContractClient<'_> {
        AttestationContractClient::new(&self.env, &self.contract_id)
    }

    fn issue_kyc(&self) -> u32 {
        self.client().issue_attestation(
            &self.issuer,
            &self.subject,
            &Symbol::new(&self.env, "kyc_verified"),
            &commit(&self.env, "passport:AB123", "s3cret"),
            &(NOW + 365 * 24 * HOUR),
        )
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn constructor_binds_admin_and_reads_are_total() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    let admin = Address::generate(&env);
    let subject = Address::generate(&env);

    let contract_id =
        env.register(AttestationContract, AttestationContractArgs::__constructor(&admin));
    let client = AttestationContractClient::new(&env, &contract_id);

    // The contract exists with the given admin; reads are total (no panic).
    assert!(!client.verify(&subject, &Symbol::new(&env, "kyc_verified")));
    let err = client.try_get_attestation(&1).unwrap_err().unwrap();
    assert_eq!(err, AttestationError::NotFound);
}

// ---------------------------------------------------------------------------
// Issuer registry
// ---------------------------------------------------------------------------

#[test]
fn issuer_registry_add_and_remove() {
    let fx = Fixture::new();
    assert!(fx.client().is_issuer(&fx.issuer));
    assert!(!fx.client().is_issuer(&fx.admin)); // admin is not automatically an issuer

    fx.client().remove_issuer(&fx.issuer);
    assert!(!fx.client().is_issuer(&fx.issuer));

    fx.client().add_issuer(&fx.issuer);
    assert!(fx.client().is_issuer(&fx.issuer));
}

#[test]
fn non_admin_cannot_manage_issuer_registry() {
    // Deliberately no mock_all_auths: with no authorization configured, an
    // arbitrary caller cannot act as admin — `admin.require_auth()` fails at
    // the host layer and the contract body never executes.
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let newcomer = Address::generate(&env);

    // Constructor auth is auto-mocked during register, so initialization works.
    let contract_id =
        env.register(AttestationContract, AttestationContractArgs::__constructor(&admin));
    let client = AttestationContractClient::new(&env, &contract_id);

    assert!(client.try_add_issuer(&newcomer).is_err());
    assert!(client.try_remove_issuer(&stranger).is_err());

    // The admin path (with authorization satisfied) is covered by the other
    // tests via the mocked-auth fixture.
}

#[test]
fn removing_unknown_issuer_errors() {
    let fx = Fixture::new();
    let err = fx
        .client()
        .try_remove_issuer(&Address::generate(&fx.env))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::InvalidIssuer);
}

// ---------------------------------------------------------------------------
// Issuance
// ---------------------------------------------------------------------------

#[test]
fn issuer_can_issue_and_ids_are_monotonic() {
    let fx = Fixture::new();
    let id1 = fx.issue_kyc();
    assert_eq!(id1, 1);

    let other = Address::generate(&fx.env);
    let id2 = fx.client().issue_attestation(
        &fx.issuer,
        &other,
        &Symbol::new(&fx.env, "accredited_investor"),
        &commit(&fx.env, "net-worth", "salt"),
        &(NOW + HOUR),
    );
    assert_eq!(id2, 2);
}

#[test]
fn non_issuer_cannot_issue() {
    let fx = Fixture::new();
    let stranger = Address::generate(&fx.env);
    let err = fx
        .client()
        .try_issue_attestation(
            &stranger,
            &fx.subject,
            &Symbol::new(&fx.env, "kyc_verified"),
            &commit(&fx.env, "v", "s"),
            &(NOW + HOUR),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::Unauthorized);
}

#[test]
fn issue_rejects_past_expiry() {
    let fx = Fixture::new();
    let err = fx
        .client()
        .try_issue_attestation(
            &fx.issuer,
            &fx.subject,
            &Symbol::new(&fx.env, "kyc_verified"),
            &commit(&fx.env, "v", "s"),
            &NOW,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::InvalidExpiry);
}

#[test]
fn issue_rejects_zero_commitment() {
    let fx = Fixture::new();
    let zero = BytesN::from_array(&fx.env, &[0u8; 32]);
    let err = fx
        .client()
        .try_issue_attestation(
            &fx.issuer,
            &fx.subject,
            &Symbol::new(&fx.env, "kyc_verified"),
            &zero,
            &(NOW + HOUR),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::InvalidClaim);
}

#[test]
fn duplicate_active_claim_type_is_rejected() {
    let fx = Fixture::new();
    fx.issue_kyc();

    let err = fx
        .client()
        .try_issue_attestation(
            &fx.issuer,
            &fx.subject,
            &Symbol::new(&fx.env, "kyc_verified"),
            &commit(&fx.env, "other", "salt"),
            &(NOW + HOUR),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::AlreadyIssued);
}

#[test]
fn reissue_is_allowed_after_revocation() {
    let fx = Fixture::new();
    let id1 = fx.issue_kyc();
    fx.client().revoke(&fx.issuer, &id1);

    let id2 = fx.client().issue_attestation(
        &fx.issuer,
        &fx.subject,
        &Symbol::new(&fx.env, "kyc_verified"),
        &commit(&fx.env, "new-claim", "salt"),
        &(NOW + HOUR),
    );
    assert_eq!(id2, 2);
    assert!(fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));

    // The revoked record is retained for auditability.
    let old = fx.client().get_attestation(&id1);
    assert!(old.revoked);
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

#[test]
fn verify_passes_for_active_attestation() {
    let fx = Fixture::new();
    fx.issue_kyc();
    assert!(fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));
}

#[test]
fn verify_fails_when_no_attestation_exists() {
    let fx = Fixture::new();
    assert!(!fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));
}

#[test]
fn verify_fails_on_claim_type_mismatch() {
    let fx = Fixture::new();
    fx.issue_kyc();
    assert!(!fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "accredited_investor")));
}

#[test]
fn verify_fails_on_subject_substitution() {
    let fx = Fixture::new();
    fx.issue_kyc();
    let attacker = Address::generate(&fx.env);
    assert!(!fx.client().verify(&attacker, &Symbol::new(&fx.env, "kyc_verified")));
}

#[test]
fn verify_fails_after_expiry() {
    let fx = Fixture::new();
    fx.client().issue_attestation(
        &fx.issuer,
        &fx.subject,
        &Symbol::new(&fx.env, "kyc_verified"),
        &commit(&fx.env, "v", "s"),
        &(NOW + HOUR),
    );
    assert!(fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));

    fx.env.ledger().set_timestamp(NOW + HOUR + 1);
    assert!(!fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));
}

#[test]
fn verify_fails_after_revocation() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();
    assert!(fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));

    fx.client().revoke(&fx.issuer, &id);
    assert!(!fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));
}

// ---------------------------------------------------------------------------
// Revocation
// ---------------------------------------------------------------------------

#[test]
fn revoke_is_persistent_and_visible_in_record() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();

    fx.client().revoke(&fx.issuer, &id);
    let record = fx.client().get_attestation(&id);
    assert!(record.revoked);
}

#[test]
fn revoke_by_admin_is_allowed() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();
    fx.client().revoke(&fx.admin, &id);
    assert!(!fx.client().verify(&fx.subject, &Symbol::new(&fx.env, "kyc_verified")));
}

#[test]
fn unrelated_account_cannot_revoke() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();
    let stranger = Address::generate(&fx.env);

    let err = fx
        .client()
        .try_revoke(&stranger, &id)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::Unauthorized);
}

#[test]
fn revoke_unknown_attestation_errors() {
    let fx = Fixture::new();
    let err = fx
        .client()
        .try_revoke(&fx.issuer, &999)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::NotFound);
}

#[test]
fn double_revoke_errors() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();
    fx.client().revoke(&fx.issuer, &id);

    let err = fx
        .client()
        .try_revoke(&fx.issuer, &id)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::Revoked);
}

// ---------------------------------------------------------------------------
// Read interface
// ---------------------------------------------------------------------------

#[test]
fn get_attestation_returns_full_record() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();

    let record: Attestation = fx.client().get_attestation(&id);
    assert_eq!(record.id, id);
    assert_eq!(record.subject, fx.subject);
    assert_eq!(record.claim_type, Symbol::new(&fx.env, "kyc_verified"));
    assert_eq!(record.claim_hash, commit(&fx.env, "passport:AB123", "s3cret"));
    assert_eq!(record.issuer, fx.issuer);
    assert_eq!(record.issued_at, NOW);
    assert_eq!(record.expiry, NOW + 365 * 24 * HOUR);
    assert!(!record.revoked);
}

#[test]
fn get_unknown_attestation_errors() {
    let fx = Fixture::new();
    let err = fx
        .client()
        .try_get_attestation(&42)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, AttestationError::NotFound);
}

// ---------------------------------------------------------------------------
// Selective disclosure
// ---------------------------------------------------------------------------

#[test]
fn disclosure_matches_claim_and_salt() {
    let fx = Fixture::new();
    fx.issue_kyc();

    assert!(fx.client().verify_claim_commitment(
        &fx.subject,
        &Symbol::new(&fx.env, "kyc_verified"),
        &Bytes::from_slice(&fx.env, b"passport:AB123"),
        &Bytes::from_slice(&fx.env, b"s3cret"),
    ));
}

#[test]
fn disclosure_rejects_wrong_value_or_salt() {
    let fx = Fixture::new();
    fx.issue_kyc();

    assert!(!fx.client().verify_claim_commitment(
        &fx.subject,
        &Symbol::new(&fx.env, "kyc_verified"),
        &Bytes::from_slice(&fx.env, b"passport:WRONG"),
        &Bytes::from_slice(&fx.env, b"s3cret"),
    ));
    assert!(!fx.client().verify_claim_commitment(
        &fx.subject,
        &Symbol::new(&fx.env, "kyc_verified"),
        &Bytes::from_slice(&fx.env, b"passport:AB123"),
        &Bytes::from_slice(&fx.env, b"wrong-salt"),
    ));
}

#[test]
fn disclosure_fails_for_revoked_or_expired() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();

    fx.client().revoke(&fx.issuer, &id);
    assert!(!fx.client().verify_claim_commitment(
        &fx.subject,
        &Symbol::new(&fx.env, "kyc_verified"),
        &Bytes::from_slice(&fx.env, b"passport:AB123"),
        &Bytes::from_slice(&fx.env, b"s3cret"),
    ));

    let fx2 = Fixture::new();
    fx2.client().issue_attestation(
        &fx2.issuer,
        &fx2.subject,
        &Symbol::new(&fx2.env, "kyc_verified"),
        &commit(&fx2.env, "v", "s"),
        &(NOW + HOUR),
    );
    fx2.env.ledger().set_timestamp(NOW + HOUR + 1);
    assert!(!fx2.client().verify_claim_commitment(
        &fx2.subject,
        &Symbol::new(&fx2.env, "kyc_verified"),
        &Bytes::from_slice(&fx2.env, b"v"),
        &Bytes::from_slice(&fx2.env, b"s"),
    ));
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[test]
fn emits_expected_events() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();

    // The environment records the events of the most recent invocation.
    let events = fx.env.events().all();
    let events = events.events();
    assert_eq!(events.len(), 1);

    let issued = events.last().unwrap();
    let ContractEventBody::V0(v0) = &issued.body;
    assert_eq!(v0.topics.len(), 2);
    let name_topic: Val = v0.topics.get(0).unwrap().try_into_val(&fx.env).unwrap();
    let name = Symbol::try_from_val(&fx.env, &name_topic).unwrap();
    assert_eq!(name, Symbol::new(&fx.env, "attestation_issued"));
    let id_topic: Val = v0.topics.get(1).unwrap().try_into_val(&fx.env).unwrap();
    let topic_id = u32::try_from_val(&fx.env, &id_topic).unwrap();
    assert_eq!(topic_id, id);

    // Data is a self-describing map keyed by field name: subject,
    // claim_type, issuer, expiry.
    let ScVal::Map(map) = &v0.data else {
        panic!("expected map data, got {:?}", v0.data);
    };
    assert_eq!(map.as_ref().unwrap().0.len(), 4);
}

#[test]
fn emits_revocation_event() {
    let fx = Fixture::new();
    let id = fx.issue_kyc();
    fx.client().revoke(&fx.issuer, &id);

    let events = fx.env.events().all();
    let events = events.events();
    let revoked = events.last().unwrap();
    let ContractEventBody::V0(v0) = &revoked.body;
    assert_eq!(v0.topics.len(), 2);
    let name_topic: Val = v0.topics.get(0).unwrap().try_into_val(&fx.env).unwrap();
    let name = Symbol::try_from_val(&fx.env, &name_topic).unwrap();
    assert_eq!(name, Symbol::new(&fx.env, "attestation_revoked"));
    let id_topic: Val = v0.topics.get(1).unwrap().try_into_val(&fx.env).unwrap();
    let topic_id = u32::try_from_val(&fx.env, &id_topic).unwrap();
    assert_eq!(topic_id, id);
}