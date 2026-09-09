#![no_std]

//! # Attestation Escrow Contract
//!
//! A Soroban escrow that holds a Stellar asset (USDC or any
//! Stellar-Asset-Contract–compatible token) and releases the funds to a
//! fixed beneficiary **only when the attestation contract confirms** that
//! the escrow's `subject` holds an active, unrevoked attestation for the
//! configured `claim_type`.
//!
//! ## Critical architectural property
//!
//! The release decision is made **on-chain by the escrow itself**: the
//! `release()` entrypoint calls [`AttestationContractClient::verify`]
//! cross-contract and refuses to move funds when verification fails. No
//! off-chain code can instruct the escrow to release — an application can
//! only *trigger* `release()`; the escrow independently re-checks the
//! attestation at execution time.
//!
//! ## Lifecycle
//!
//! 1. `deposit(from, amount)` — `from` transfers the asset into the escrow.
//! 2. `release(amount)` — anyone may trigger it; funds move to the
//!    beneficiary only if `verify(subject, claim_type)` succeeds.
//! 3. `withdraw(from, amount)` — a depositor may claw back their own
//!    deposit before any release occurs (e.g. the attestation can never be
//!    satisfied). After a release the escrow is closed.
//!
//! ## Threat model
//!
//! - **Forged release**: funds only move through `release()`, which is
//!   gated by the attestation contract's `verify()` — a revoked, expired,
//!   or missing attestation blocks the transfer with
//!   [`EscrowError::AttestationNotVerified`].
//! - **Theft via withdraw**: `withdraw()` is limited to each depositor's
//!   own recorded deposit and is disabled after the first release.
//! - **Front-running**: the beneficiary is fixed at construction and the
//!   asset is the configured SAC token, so a front-runner cannot redirect
//!   funds.
//! - **Stuck funds**: no privileged movement functions exist; each
//!   depositor can claw back their own deposit before release, so the
//!   escrow stays non-custodial and funds are never locked in.

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype, token,
    Address, Env, Map, MuxedAddress, Symbol,
};

/// Minimal interface to the attestation contract's `verify()` entrypoint.
///
/// Defined locally with [`contractclient`] (instead of depending on the
/// attestation crate) so the escrow's wasm only contains the cross-contract
/// call — the attestation contract's code is never linked in, keeping each
/// contract's wasm small and free of duplicate exported symbols.
#[contractclient(name = "AttestationClient")]
pub trait AttestationInterface {
    /// Returns `true` when `subject` holds an active, unrevoked attestation
    /// for `claim_type`.
    fn verify(env: Env, subject: Address, claim_type: Symbol) -> bool;
}

/// Ledgers per day. Stellar ledgers close every ~5 seconds on mainnet and
/// testnet.
pub const LEDGERS_PER_DAY: u32 = 17_280;
/// TTL extension window for instance storage (31 days).
pub const INSTANCE_TTL_EXTEND: u32 = 31 * LEDGERS_PER_DAY;
/// TTL threshold below which instance storage is re-extended.
pub const INSTANCE_TTL_THRESHOLD: u32 = INSTANCE_TTL_EXTEND / 2;
/// TTL extension window for persistent storage (365 days).
pub const PERSISTENT_TTL_EXTEND: u32 = 365 * LEDGERS_PER_DAY;
/// TTL threshold below which persistent storage is re-extended.
pub const PERSISTENT_TTL_THRESHOLD: u32 = PERSISTENT_TTL_EXTEND / 2;

/// Storage keys. Config and lifecycle flags live in instance storage;
/// balances live in persistent storage so they survive TTL rollover.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance: the admin account that configured the escrow.
    Admin,
    /// Instance: the SAC-compatible token held by the escrow.
    Asset,
    /// Instance: the attestation contract consulted by `release()`.
    AttestationContract,
    /// Instance: the subject whose attestation gates the release.
    Subject,
    /// Instance: the claim type the subject must hold.
    ClaimType,
    /// Instance: the fixed recipient of released funds.
    Beneficiary,
    /// Instance: true once any release has happened; closes the escrow.
    Released,
    /// Persistent: total asset balance held by the escrow.
    Balance,
    /// Persistent: per-depositor balances for clawback.
    Deposits,
}

/// Escrow error codes. These values are part of the public interface and
/// are depended on by SDKs and off-chain clients — keep stable.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum EscrowError {
    /// The escrow has not been initialized.
    NotInitialized = 1,
    /// Caller is not authorized for this operation.
    Unauthorized = 2,
    /// The requested amount is zero or negative.
    InvalidAmount = 3,
    /// The escrow (or the depositor) holds less than the requested amount.
    InsufficientBalance = 4,
    /// The subject does not hold a valid attestation for the required claim.
    AttestationNotVerified = 5,
    /// The escrow is closed because a release has already occurred.
    EscrowClosed = 6,
}

/// Full escrow configuration, returned to off-chain clients for display.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowConfig {
    pub admin: Address,
    pub asset: Address,
    pub attestation_contract: Address,
    pub subject: Address,
    pub claim_type: Symbol,
    pub beneficiary: Address,
    pub released: bool,
}

/// Emitted when `from` deposits `amount` into the escrow.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deposited {
    #[topic]
    pub from: Address,
    pub amount: i128,
    pub total: i128,
}

/// Emitted when funds are released to the beneficiary after a successful
/// on-chain verification.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Released {
    pub beneficiary: Address,
    pub amount: i128,
    pub remaining: i128,
}

/// Emitted when a depositor claws back part of their deposit.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdrawn {
    #[topic]
    pub from: Address,
    pub amount: i128,
    pub remaining: i128,
}

#[contract]
pub struct AttestationEscrow;

#[contractimpl]
impl AttestationEscrow {
    /// Constructor: binds the escrow configuration atomically at deploy
    /// time. `admin` must authorize the deployment.
    pub fn __constructor(
        env: Env,
        admin: Address,
        asset: Address,
        attestation_contract: Address,
        subject: Address,
        claim_type: Symbol,
        beneficiary: Address,
    ) {
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Asset, &asset);
        env.storage()
            .instance()
            .set(&DataKey::AttestationContract, &attestation_contract);
        env.storage().instance().set(&DataKey::Subject, &subject);
        env.storage()
            .instance()
            .set(&DataKey::ClaimType, &claim_type);
        env.storage()
            .instance()
            .set(&DataKey::Beneficiary, &beneficiary);
        env.storage().instance().set(&DataKey::Released, &false);
        extend_instance_ttl(&env);
    }

    /// Deposit `amount` of the escrow's asset from `from`.
    ///
    /// Only the depositor may fund their own balance; a single escrow
    /// instance can hold deposits from several depositors, and each can
    /// claw back their own share before release.
    pub fn deposit(env: Env, from: Address, amount: i128) -> Result<(), EscrowError> {
        Self::require_initialized(&env)?;
        from.require_auth();

        if amount <= 0 {
            return Err(EscrowError::InvalidAmount);
        }
        if Self::released_flag(&env) {
            return Err(EscrowError::EscrowClosed);
        }

        let asset = Self::asset(&env)?;
        let current = env.current_contract_address();
        token::Client::new(&env, &asset).transfer(&from, MuxedAddress::from(&current), &amount);

        let mut deposits: Map<Address, i128> = env
            .storage()
            .persistent()
            .get(&DataKey::Deposits)
            .unwrap_or(Map::new(&env));
        let held = deposits.get(from.clone()).unwrap_or(0);
        deposits.set(from.clone(), held + amount);
        env.storage()
            .persistent()
            .set(&DataKey::Deposits, &deposits);
        extend_persistent_ttl(&env, &DataKey::Deposits);

        let total = Self::balance(&env) + amount;
        env.storage().persistent().set(&DataKey::Balance, &total);
        extend_persistent_ttl(&env, &DataKey::Balance);
        extend_instance_ttl(&env);

        env.events().publish_event(&Deposited {
            from,
            amount,
            total,
        });
        Ok(())
    }

    /// Release `amount` to the beneficiary.
    ///
    /// Anyone may trigger this; the escrow itself re-checks the
    /// attestation **on-chain** via a cross-contract `verify()` call and
    /// fails with [`EscrowError::AttestationNotVerified`] when the subject
    /// does not hold an active, unrevoked attestation. The first release
    /// closes the escrow.
    pub fn release(env: Env, amount: i128) -> Result<(), EscrowError> {
        Self::require_initialized(&env)?;

        if amount <= 0 {
            return Err(EscrowError::InvalidAmount);
        }
        if Self::released_flag(&env) {
            return Err(EscrowError::EscrowClosed);
        }
        let balance = Self::balance(&env);
        if amount > balance {
            return Err(EscrowError::InsufficientBalance);
        }

        // The gating decision is made here, on-chain, by the attestation
        // contract — never by an off-chain caller.
        let attestation_contract = Self::attestation_contract(&env)?;
        let subject = Self::subject(&env)?;
        let claim_type = Self::claim_type(&env)?;
        if !AttestationClient::new(&env, &attestation_contract).verify(&subject, &claim_type) {
            return Err(EscrowError::AttestationNotVerified);
        }

        let beneficiary = Self::beneficiary(&env)?;
        let asset = Self::asset(&env)?;
        let current = env.current_contract_address();
        token::Client::new(&env, &asset).transfer(
            &current,
            MuxedAddress::from(&beneficiary),
            &amount,
        );

        let remaining = balance - amount;
        env.storage()
            .persistent()
            .set(&DataKey::Balance, &remaining);
        extend_persistent_ttl(&env, &DataKey::Balance);
        env.storage().instance().set(&DataKey::Released, &true);
        extend_instance_ttl(&env);

        env.events().publish_event(&Released {
            beneficiary,
            amount,
            remaining,
        });
        Ok(())
    }

    /// Claw back `amount` of `from`'s own deposit. Only the depositor may
    /// withdraw, only up to their recorded share, and only before any
    /// release has closed the escrow.
    pub fn withdraw(env: Env, from: Address, amount: i128) -> Result<(), EscrowError> {
        Self::require_initialized(&env)?;
        from.require_auth();

        if amount <= 0 {
            return Err(EscrowError::InvalidAmount);
        }
        if Self::released_flag(&env) {
            return Err(EscrowError::EscrowClosed);
        }

        let mut deposits: Map<Address, i128> = env
            .storage()
            .persistent()
            .get(&DataKey::Deposits)
            .unwrap_or(Map::new(&env));
        let held = deposits.get(from.clone()).unwrap_or(0);
        if amount > held {
            return Err(EscrowError::InsufficientBalance);
        }

        let asset = Self::asset(&env)?;
        let current = env.current_contract_address();
        token::Client::new(&env, &asset).transfer(&current, MuxedAddress::from(&from), &amount);

        let new_held = held - amount;
        if new_held == 0 {
            deposits.remove(from.clone());
        } else {
            deposits.set(from.clone(), new_held);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Deposits, &deposits);
        extend_persistent_ttl(&env, &DataKey::Deposits);

        let total = Self::balance(&env) - amount;
        env.storage().persistent().set(&DataKey::Balance, &total);
        extend_persistent_ttl(&env, &DataKey::Balance);
        extend_instance_ttl(&env);

        env.events().publish_event(&Withdrawn {
            from,
            amount,
            remaining: total,
        });
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Read-only helpers
    // ---------------------------------------------------------------------

    /// Total asset balance currently held by the escrow.
    pub fn get_balance(env: Env) -> i128 {
        Self::balance(&env)
    }

    /// The amount `address` has deposited and can still claw back.
    pub fn get_deposit(env: Env, address: Address) -> i128 {
        env.storage()
            .persistent()
            .get::<DataKey, Map<Address, i128>>(&DataKey::Deposits)
            .unwrap_or(Map::new(&env))
            .get(address)
            .unwrap_or(0)
    }

    /// Returns `true` once any release has occurred (escrow closed).
    pub fn is_released(env: Env) -> bool {
        Self::released_flag(&env)
    }

    /// Full escrow configuration for off-chain display.
    pub fn config(env: Env) -> Result<EscrowConfig, EscrowError> {
        Ok(EscrowConfig {
            admin: Self::admin(&env)?,
            asset: Self::asset(&env)?,
            attestation_contract: Self::attestation_contract(&env)?,
            subject: Self::subject(&env)?,
            claim_type: Self::claim_type(&env)?,
            beneficiary: Self::beneficiary(&env)?,
            released: Self::released_flag(&env),
        })
    }
}

impl AttestationEscrow {
    fn admin(env: &Env) -> Result<Address, EscrowError> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(EscrowError::NotInitialized)
    }

    fn asset(env: &Env) -> Result<Address, EscrowError> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Asset)
            .ok_or(EscrowError::NotInitialized)
    }

    fn attestation_contract(env: &Env) -> Result<Address, EscrowError> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::AttestationContract)
            .ok_or(EscrowError::NotInitialized)
    }

    fn subject(env: &Env) -> Result<Address, EscrowError> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Subject)
            .ok_or(EscrowError::NotInitialized)
    }

    fn claim_type(env: &Env) -> Result<Symbol, EscrowError> {
        env.storage()
            .instance()
            .get::<DataKey, Symbol>(&DataKey::ClaimType)
            .ok_or(EscrowError::NotInitialized)
    }

    fn beneficiary(env: &Env) -> Result<Address, EscrowError> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Beneficiary)
            .ok_or(EscrowError::NotInitialized)
    }

    fn released_flag(env: &Env) -> bool {
        env.storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::Released)
            .unwrap_or(false)
    }

    fn balance(env: &Env) -> i128 {
        env.storage()
            .persistent()
            .get::<DataKey, i128>(&DataKey::Balance)
            .unwrap_or(0)
    }

    fn require_initialized(env: &Env) -> Result<(), EscrowError> {
        if env.storage().instance().has(&DataKey::Admin) {
            Ok(())
        } else {
            Err(EscrowError::NotInitialized)
        }
    }
}

/// Keep instance storage alive (31 days).
fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND);
}

/// Keep a persistent entry alive (365 days).
fn extend_persistent_ttl<K>(env: &Env, key: &K)
where
    K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>,
{
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_TTL_THRESHOLD, PERSISTENT_TTL_EXTEND);
}
