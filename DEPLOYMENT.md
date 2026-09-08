# Deployment

End-to-end guide for building, deploying, and exercising the attestation
contract. Every command below was executed against a live testnet deployment
during development (contract id in `deployments/testnet.json`).

## Prerequisites

| Tool | Version | Install |
|---|---|---|
| Rust | stable (see `rust-toolchain.toml`) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| stellar CLI | v28+ | prebuilt binaries from [stellar/stellar-cli](https://github.com/stellar/stellar-cli/releases) |

The `wasm32v1-none` target is required by the Soroban environment for
Rust 1.82+ (the older `wasm32-unknown-unknown` enables
reference-types/multi-value that the VM rejects):

```bash
rustup target add wasm32v1-none
```

## 1. Create and fund a testnet account

```bash
stellar keys generate --fund testnet-issuer
# ✅ Key saved with alias testnet-issuer
# ✅ Account testnet-issuer funded on "Test SDF Network ; September 2015"

stellar keys public-key testnet-issuer
# GAWNWSHQDTYETV6RS5AAQED5V27HAQL7MQPZMHKY65JAPFWW35WE7RV2
```

`--fund` tops up the account via the testnet faucet (10000 XLM). Confirm the
network is configured: `stellar network ls` should include `testnet`
(`https://soroban-testnet.stellar.org`).

## 2. Build and optimize

```bash
make wasm            # -> target/wasm32v1-none/release/attestation_contract.wasm (26 KB)
make optimize        # -> target/optimized/attestation_contract.wasm (20 KB)
```

## 3. Deploy

The constructor requires the admin address. Pass it after the `--` separator:

```bash
stellar contract deploy \
  --network testnet \
  --source testnet-issuer \
  --wasm target/optimized/attestation_contract.wasm \
  -- --admin GAWNWSHQDTYETV6RS5AAQED5V27HAQL7MQPZMHKY65JAPFWW35WE7RV2
# ✅ Deployed!
# CB2MGYTG6MIIDYWVB5BV4FLEF7KRDEF556JMVZXSZ22XALB7MUC7LU2S
```

or, one command:

```bash
STELLAR_SOURCE_ACCOUNT=testnet-issuer make deploy-testnet
```

which also writes `deployments/testnet.json`.

> The CLI generates an *implicit CLI* from the contract schema — argument
> values are passed **bare** (no type prefixes): `--claim_type kyc_verified`,
> `--claim_hash <64 hex chars>`, `--expiry <unix seconds>`.
> Run `stellar contract invoke ... --id <ID> -- issue_attestation --help` to
> see the exact formats.

## 4. Exercise the protocol

```bash
ID=CB2MGYTG6MIIDYWVB5BV4FLEF7KRDEF556JMVZXSZ22XALB7MUC7LU2S
ISSUER=$(stellar keys public-key testnet-issuer)

# Register the issuer (admin only)
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- add_issuer --issuer $ISSUER

# Issue: commitment = sha256("passport:AB123" + "s3cret")
HASH=$(python3 -c "import hashlib;print(hashlib.sha256(b'passport:AB123s3cret').hexdigest())")
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- issue_attestation --issuer $ISSUER --subject $ISSUER \
  --claim_type kyc_verified --claim_hash $HASH --expiry 1900000000
# -> 1

# Verify
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- verify --subject $ISSUER --claim_type kyc_verified
# -> true

# Selective disclosure
VAL=$(python3 -c "print('passport:AB123'.encode().hex())")
SALT=$(python3 -c "print('s3cret'.encode().hex())")
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- verify_claim_commitment --subject $ISSUER --claim_type kyc_verified \
  --claim_value $VAL --salt $SALT
# -> true (wrong value -> false)

# Revoke, then verify again
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- revoke --caller $ISSUER --attestation_id 1
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- verify --subject $ISSUER --claim_type kyc_verified
# -> false
```

## 5. Testnet → future production

1. Deploy the same optimized wasm to a funded mainnet account
   (`stellar contract deploy --network public ...`).
2. Point the backend SDK, the frontend, and any escrow contracts at the new
   contract id (each repo reads `deployments/*.json`).
3. Move the admin/issuer keys into production custody (vault / hardware
   wallet).
4. Re-run the smoke-test sequence from §4 on mainnet before enabling real
   traffic.

## 6. Verification of a deployment

To confirm a deployed contract is *this* contract:

```bash
stellar contract invoke --network testnet --source testnet-issuer --id $ID \
  -- is_issuer --address GAWNWSHQDTYETV6RS5AAQED5V27HAQL7MQPZMHKY65JAPFWW35WE7RV2
# -> false (or true if registered) — an unrecognized function name will error
```

Compare the deployed wasm hash with a locally built one using
`stellar contract install`/`stellar contract inspect` tooling, or simply
re-run the lifecycle above — forged or foreign contracts will not expose the
documented functions/events.