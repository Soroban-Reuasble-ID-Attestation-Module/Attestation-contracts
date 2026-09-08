#!/usr/bin/env bash
# Invoke a function on the deployed testnet attestation contract.
#
# Usage:
#   ./scripts/invoke-testnet.sh --fn verify --arg 'address:G...' --arg 'symbol:kyc_verified'
#
# Example:
#   STELLAR_SOURCE_ACCOUNT=<secret-key> ./scripts/invoke-testnet.sh \
#     --id C...CONTRACTID --fn verify --arg 'address:GCX...' --arg 'symbol:kyc_verified'
set -euo pipefail

cd "$(dirname "$0")/.."

: "${STELLAR_SOURCE_ACCOUNT:?Set STELLAR_SOURCE_ACCOUNT to a funded testnet secret key}"
NETWORK="${STELLAR_NETWORK:-testnet}"

# Resolve the contract id: explicit --id flag, or deployments/testnet.json.
ARGS=("$@")
CONTRACT_ID=""
for i in "${!ARGS[@]}"; do
  if [[ "${ARGS[$i]}" == "--id" ]]; then
    CONTRACT_ID="${ARGS[$((i + 1))]}"
    unset 'ARGS[i]'
    unset 'ARGS[i+1]'
  fi
done

if [[ -z "$CONTRACT_ID" ]] && [[ -f deployments/testnet.json ]]; then
  CONTRACT_ID=$(python3 -c "import json;print(json.load(open('deployments/testnet.json'))['attestation_contract'])")
fi
if [[ -z "$CONTRACT_ID" ]]; then
  echo "No contract id: pass --id <CONTRACT_ID> or deploy first (deployments/testnet.json)." >&2
  exit 1
fi

stellar contract invoke \
  --network "$NETWORK" \
  --source "$STELLAR_SOURCE_ACCOUNT" \
  --id "$CONTRACT_ID" \
  "${ARGS[@]}"