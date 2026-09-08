//! Storage access helpers and TTL management.
//!
//! Stellar ledger entries have a finite TTL measured in ledgers. Every read
//! or write extends the TTL of the touched entries so live attestations and
//! registry state survive long enough to be queried, verified, and revoked.
//! Instance storage (admin, id counter) is kept alive for 31 days;
//! persistent attestation state is kept alive for one year.

use soroban_sdk::{Env, IntoVal, Val};

/// Ledgers per day. Stellar ledgers close every ~5 seconds on mainnet and
/// testnet.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// TTL extension window for instance storage (31 days).
pub const INSTANCE_TTL_EXTEND: u32 = 31 * LEDGERS_PER_DAY;

/// TTL extension window for persistent storage (365 days).
pub const PERSISTENT_TTL_EXTEND: u32 = 365 * LEDGERS_PER_DAY;

/// Threshold below which instance storage is re-extended (half the window).
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL_EXTEND / 2;

/// Threshold below which persistent storage is re-extended (half the window).
pub const PERSISTENT_TTL_THRESHOLD: u32 = PERSISTENT_TTL_EXTEND / 2;

/// Keep instance storage alive.
pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

/// Keep a persistent entry alive.
pub fn extend_persistent_ttl<K>(env: &Env, key: &K)
where
    K: IntoVal<Env, Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND);
}
