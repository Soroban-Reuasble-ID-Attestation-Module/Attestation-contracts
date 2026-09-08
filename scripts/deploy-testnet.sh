#!/usr/bin/env bash
# Deploy the attestation contract to Stellar testnet and persist the
# resulting contract id to deployments/testnet.json.
#
# Requirements:
#   - `stellar` CLI v21+ installed and authenticated
#   - STELLAR_SOURCE_ACCOUNT: secret key of a funded testnet account
#     (create + fund one with `stellar keys generate --fund testnet-issuer`)
#   - The testnet network configured in stellar CLI (`stellar network add ...`)
#     or the default testnet config
#
# Usage:
#   STELLAR_SOURCE_ACCOUNT=<secret-key> ./scripts/deploy-testnet.sh
set -euo pipefail

cd "$(dirname "$0")/.."

: "${STELLAR_SOURCE_ACCOUNT:?Set STELLAR_SOURCE_ACCOUNT to a funded testnet secret key}"

NETWORK="${STELLAR_NETWORK:-testnet}"
RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"

echo "Building optimized wasm..."
./scripts/build.sh
./scripts/optimize.sh

echo "Deploying to ${NETWORK}..."
OUTPUT=$(stellar contract deploy \
  --network "$NETWORK" \
  --source "$STELLAR_SOURCE_ACCOUNT" \
  --wasm target/optimized/attestation_contract.wasm \
  -- --admin "$(stellar keys address --network "$NETWORK" "$STELLAR_SOURCE_ACCOUNT")")

CONTRACT_ID=$(echo "$OUTPUT" | tail -1)
if [[ ! "$CONTRACT_ID" =~ ^C[0-9A-Z]{55}$ ]]; then
  echo "Unexpected deploy output: $OUTPUT" >&2
  exit 1
fi

mkdir -p deployments
cat > deployments/testnet.json <<EOF
{
  "network": "${NETWORK}",
  "rpc_url": "${RPC_URL}",
  "attestation_contract": "${CONTRACT_ID}"
}
EOF

echo "Deployed attestation contract: ${CONTRACT_ID}"
echo "Addresses written to deployments/testnet.json"