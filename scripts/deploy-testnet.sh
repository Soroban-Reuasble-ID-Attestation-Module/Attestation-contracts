#!/usr/bin/env bash
# Deploy the attestation contract to Stellar testnet and persist the
# resulting contract id to deployments/testnet.json.
#
# Requirements:
#   - `stellar` CLI v28+ installed and authenticated
#   - STELLAR_SOURCE_ACCOUNT: a funded testnet identity name
#     (create + fund one with:  stellar keys generate --fund myissuer)
#   - The testnet network configured in the stellar CLI
#     (stellar network ls | grep testnet)
#
# Usage:
#   STELLAR_SOURCE_ACCOUNT=myissuer ./scripts/deploy-testnet.sh
set -euo pipefail

cd "$(dirname "$0")/.."

: "${STELLAR_SOURCE_ACCOUNT:?Set STELLAR_SOURCE_ACCOUNT to a funded testnet identity name, e.g. created with: stellar keys generate --fund myissuer}"

NETWORK="${STELLAR_NETWORK:-testnet}"

echo "Building optimized wasm..."
./scripts/build.sh
./scripts/optimize.sh

ADMIN=$(stellar keys public-key "$STELLAR_SOURCE_ACCOUNT")
echo "Deploying to ${NETWORK} with admin ${ADMIN}..."

OUTPUT=$(stellar contract deploy \
  --network "$NETWORK" \
  --source "$STELLAR_SOURCE_ACCOUNT" \
  --wasm target/optimized/attestation_contract.wasm \
  -- --admin "$ADMIN")

CONTRACT_ID=$(echo "$OUTPUT" | tail -1)
if [[ ! "$CONTRACT_ID" =~ ^C[0-9A-Z]{55}$ ]]; then
  echo "Unexpected deploy output: $OUTPUT" >&2
  exit 1
fi

mkdir -p deployments
cat > deployments/testnet.json <<EOF
{
  "network": "${NETWORK}",
  "rpc_url": "https://soroban-testnet.stellar.org",
  "horizon_url": "https://horizon-testnet.stellar.org",
  "attestation_contract": "${CONTRACT_ID}"
}
EOF

echo "Deployed attestation contract: ${CONTRACT_ID}"
echo "Addresses written to deployments/testnet.json"