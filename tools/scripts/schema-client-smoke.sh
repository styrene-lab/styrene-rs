#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

MANIFEST_PATH="docs/schemas/sdk/v2/clients/client-generation-manifest.json"
SMOKE_PATH="docs/schemas/sdk/v2/clients/smoke-requests.json"
REPORT_PATH="target/interop/schema-client-smoke-report.txt"

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "missing manifest: $MANIFEST_PATH" >&2
  exit 1
fi

if [[ ! -f "$SMOKE_PATH" ]]; then
  echo "missing smoke vectors: $SMOKE_PATH" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for schema-client-smoke.sh" >&2
  exit 1
fi

required_languages=("go" "javascript" "python")

for language in "${required_languages[@]}"; do
  if ! jq -e --arg lang "$language" '.targets[] | select(.language == $lang)' "$MANIFEST_PATH" >/dev/null; then
    echo "manifest missing client target language: $language" >&2
    exit 1
  fi
done

while IFS= read -r schema_path; do
  if [[ ! -f "$schema_path" ]]; then
    echo "manifest references missing schema: $schema_path" >&2
    exit 1
  fi
done < <(jq -r '.required_schemas[]' "$MANIFEST_PATH")

for language in "${required_languages[@]}"; do
  if ! jq -e --arg lang "$language" '.smoke_vectors[] | select(.language == $lang)' "$SMOKE_PATH" >/dev/null; then
    echo "smoke vectors missing language: $language" >&2
    exit 1
  fi
done

mkdir -p "$(dirname "$REPORT_PATH")"
{
  echo "# Schema Client Smoke Report"
  echo "manifest=$MANIFEST_PATH"
  echo "smoke_vectors=$SMOKE_PATH"
  echo "target_count=$(jq '.targets | length' "$MANIFEST_PATH")"
  echo "required_schema_count=$(jq '.required_schemas | length' "$MANIFEST_PATH")"
  echo "smoke_vector_count=$(jq '.smoke_vectors | length' "$SMOKE_PATH")"
  echo "status=PASS"
} > "$REPORT_PATH"
