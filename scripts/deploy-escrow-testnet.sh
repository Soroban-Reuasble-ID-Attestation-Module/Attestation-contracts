#!/usr/bin/env bash
# Deploy the attestation-gated escrow contract to Stellar testnet and append
# the resulting contract id to deployments/testnet.json.
#
# Requirements:
#   - `stellar` CLI v28+ installed and authenticated
#   - STELLAR_SOURCE_ACCOUNT: a funded testnet identity name
#     (create + fund one with:  stellar keys generate --fund myissuer)
#   - ESCROW_ASSET: address of the SAC-compatible token the escrow will hold
#     (e.g. testnet USDC, or any SAC token)
#   - ESCROW_SUBJECT: Stellar address that must hold the attestation
#   - ESCROW_CLAIM_TYPE: claim symbol the subject must hold, e.g. kyc_verified
#   - ESCROW_BENEFICIARY: Stellar address that receives released funds
#   - The attestation contract must already be deployed (deployments/testnet.json)
#
# Usage:
#   ESCROW_ASSET=C... ESCROW_SUBJECT=G... ESCROW_CLAIM_TYPE=kyc_verified \
#     ESCROW_BENEFICIARY=G... STELLAR_SOURCE_ACCOUNT=myissuer \
#     ./scripts/deploy-escrow-testnet.sh
set -euo pipefail

cd "$(dirname "$0")/.."

: "${STELLAR_SOURCE_ACCOUNT:?Set STELLAR_SOURCE_ACCOUNT to a funded testnet identity name, e.g. created with: stellar keys generate --fund myissuer}"
: "${ESCROW_ASSET:?Set ESCROW_ASSET to the SAC token address the escrow will hold}"
: "${ESCROW_SUBJECT:?Set ESCROW_SUBJECT to the subject address that must hold the attestation}"
: "${ESCROW_CLAIM_TYPE:?Set ESCROW_CLAIM_TYPE, e.g. kyc_verified}"
: "${ESCROW_BENEFICIARY:?Set ESCROW_BENEFICIARY to the beneficiary address}"

NETWORK="${STELLAR_NETWORK:-testnet}"

if [[ ! -f deployments/testnet.json ]]; then
  echo "deployments/testnet.json not found — deploy the attestation contract first (./scripts/deploy-testnet.sh)." >&2
  exit 1
fi
ATTESTATION_CONTRACT=$(python3 -c "import json;print(json.load(open('deployments/testnet.json'))['attestation_contract'])")
if [[ ! "$ATTESTATION_CONTRACT" =~ ^C[0-9A-Z]{55}$ ]]; then
  echo "Invalid attestation_contract in deployments/testnet.json." >&2
  exit 1
fi

echo "Building optimized wasm..."
./scripts/build.sh
./scripts/optimize.sh

ADMIN=$(stellar keys public-key "$STELLAR_SOURCE_ACCOUNT")
echo "Deploying escrow to ${NETWORK} with admin ${ADMIN}..."
echo "  asset=$ESCROW_ASSET attestation=$ATTESTATION_CONTRACT"
echo "  subject=$ESCROW_SUBJECT claim_type=$ESCROW_CLAIM_TYPE beneficiary=$ESCROW_BENEFICIARY"

OUTPUT=$(stellar contract deploy \
  --network "$NETWORK" \
  --source "$STELLAR_SOURCE_ACCOUNT" \
  --wasm target/optimized/attestation_escrow_contract.wasm \
  -- \
  --admin "$ADMIN" \
  --asset "$ESCROW_ASSET" \
  --attestation_contract "$ATTESTATION_CONTRACT" \
  --subject "$ESCROW_SUBJECT" \
  --claim_type "symbol:${ESCROW_CLAIM_TYPE}" \
  --beneficiary "$ESCROW_BENEFICIARY")

ESCROW_ID=$(echo "$OUTPUT" | tail -1)
if [[ ! "$ESCROW_ID" =~ ^C[0-9A-Z]{55}$ ]]; then
  echo "Unexpected deploy output: $OUTPUT" >&2
  exit 1
fi

python3 - "$ESCROW_ID" "$ESCROW_ASSET" "$ESCROW_SUBJECT" "$ESCROW_CLAIM_TYPE" "$ESCROW_BENEFICIARY" <<'PY'
import json, sys
escrow_id, asset, subject, claim_type, beneficiary = sys.argv[1:6]
with open("deployments/testnet.json") as f:
    cfg = json.load(f)
cfg["escrow_contract"] = escrow_id
cfg["escrow_asset"] = asset
cfg["escrow_subject"] = subject
cfg["escrow_claim_type"] = claim_type
cfg["escrow_beneficiary"] = beneficiary
with open("deployments/testnet.json", "w") as f:
    json.dump(cfg, f, indent=2)
    f.write("\n")
print(f"deployments/testnet.json updated with escrow_contract={escrow_id}")
PY

echo "Deployed escrow contract: ${ESCROW_ID}"