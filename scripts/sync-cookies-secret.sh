#!/usr/bin/env bash
set -euo pipefail

# Sync cookies from local directory into Kubernetes Secret.
# Cookie files are discovered using strict structure:
#   cookies/<domain>/<index>.txt
# Kubernetes Secret keys cannot contain '/', so keys are flattened to:
#   <domain>_<index>.txt
# Example:
#   cookies/youtube.com/1.txt -> key youtube.com_1.txt

SOURCE_DIR="${1:-cookies}"
NAMESPACE="${NAMESPACE:-bot}"
SECRET_NAME="${SECRET_NAME:-bot-cookies}"
KUBECTL_BIN="${KUBECTL_BIN:-kubectl}"

if [[ ! -d "${SOURCE_DIR}" ]]; then
  echo "Source directory not found: ${SOURCE_DIR}" >&2
  exit 1
fi

shopt -s nullglob
COOKIE_FILES=()
for path in "${SOURCE_DIR}"/*/*.txt; do
  COOKIE_FILES+=("${path}")
done
shopt -u nullglob

if [[ ${#COOKIE_FILES[@]} -gt 0 ]]; then
  mapfile -t COOKIE_FILES < <(printf '%s\n' "${COOKIE_FILES[@]}" | sort)
fi

if [[ ${#COOKIE_FILES[@]} -eq 0 ]]; then
  echo "No cookie files found in ${SOURCE_DIR}" >&2
  exit 1
fi

TMP_MANIFEST="$(mktemp)"
cleanup() {
  rm -f "${TMP_MANIFEST}"
}
trap cleanup EXIT

CMD=("${KUBECTL_BIN}" -n "${NAMESPACE}" create secret generic "${SECRET_NAME}" --dry-run=client -o yaml)

for abs_path in "${COOKIE_FILES[@]}"; do
  rel_path="${abs_path#${SOURCE_DIR}/}"
  domain="$(dirname "${rel_path}")"
  name="$(basename "${rel_path}")"
  if [[ "${domain}" == "." || "${name}" != *.txt ]]; then
    echo "Unsupported cookie path: ${rel_path}. Expected <domain>/<index>.txt" >&2
    exit 1
  fi
  if [[ "${domain}" == *"/"* ]]; then
    echo "Unsupported nested cookie path: ${rel_path}. Expected <domain>/<index>.txt" >&2
    exit 1
  fi
  if [[ ! "${name}" =~ ^[0-9]+\.txt$ ]]; then
    echo "Unsupported cookie filename: ${rel_path}. Expected numeric <index>.txt" >&2
    exit 1
  fi
  key="${domain}_${name}"
  CMD+=(--from-file="${key}=${abs_path}")
done

"${CMD[@]}" > "${TMP_MANIFEST}"
"${KUBECTL_BIN}" -n "${NAMESPACE}" apply -f "${TMP_MANIFEST}"

echo "Synced ${#COOKIE_FILES[@]} cookie file(s) into secret ${SECRET_NAME} in namespace ${NAMESPACE}."
