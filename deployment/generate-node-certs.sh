#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  deployment/generate-node-certs.sh --root-dir PATH --node NAME [--ip ADDR]... [--dns NAME]... [--force]

Creates a private CA on first use and issues a TLS server certificate for one downloader node.

Examples:
  deployment/generate-node-certs.sh \
    --root-dir ./tls \
    --node local \
    --ip 127.0.0.1 \
    --dns localhost

  deployment/generate-node-certs.sh \
    --root-dir ./tls \
    --node external \
    --ip 203.0.113.10

Output layout:
  <root-dir>/ca/ca.crt
  <root-dir>/ca/ca.key
  <root-dir>/ca/ca.srl
  <root-dir>/nodes/<node>/server.crt
  <root-dir>/nodes/<node>/server.key
  <root-dir>/nodes/<node>/ca.crt

Notes:
  - SANs must match exactly what the bot will connect to.
  - Keep ca.key only on the machine that issues certificates.
  - Use --force to overwrite an existing node certificate.
EOF
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing required command: $1" >&2
        exit 1
    fi
}

ROOT_DIR=""
NODE_NAME=""
FORCE=0
CA_DAYS=3650
NODE_DAYS=825
IPS=()
DNSES=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --root-dir)
            ROOT_DIR="${2:-}"
            shift 2
            ;;
        --node)
            NODE_NAME="${2:-}"
            shift 2
            ;;
        --ip)
            IPS+=("${2:-}")
            shift 2
            ;;
        --dns)
            DNSES+=("${2:-}")
            shift 2
            ;;
        --force)
            FORCE=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if [[ -z "$ROOT_DIR" || -z "$NODE_NAME" ]]; then
    echo "--root-dir and --node are required" >&2
    usage >&2
    exit 1
fi

if [[ ${#IPS[@]} -eq 0 && ${#DNSES[@]} -eq 0 ]]; then
    echo "At least one --ip or --dns SAN is required" >&2
    exit 1
fi

require_command openssl
require_command mktemp

ROOT_DIR="${ROOT_DIR%/}"
CA_DIR="$ROOT_DIR/ca"
NODE_DIR="$ROOT_DIR/nodes/$NODE_NAME"

mkdir -p "$CA_DIR" "$NODE_DIR"

CA_CERT="$CA_DIR/ca.crt"
CA_KEY="$CA_DIR/ca.key"
CA_SERIAL="$CA_DIR/ca.srl"
SERVER_KEY="$NODE_DIR/server.key"
SERVER_CSR="$NODE_DIR/server.csr"
SERVER_CERT="$NODE_DIR/server.crt"
NODE_CA_CERT="$NODE_DIR/ca.crt"

if [[ ! -f "$CA_CERT" || ! -f "$CA_KEY" ]]; then
    echo "Creating CA in $CA_DIR"
    openssl genrsa -out "$CA_KEY" 4096
    openssl req \
        -x509 \
        -new \
        -sha256 \
        -days "$CA_DAYS" \
        -key "$CA_KEY" \
        -out "$CA_CERT" \
        -subj "/CN=ytdl-tg-bot-downloader-ca"
    chmod 600 "$CA_KEY"
else
    echo "Using existing CA in $CA_DIR"
fi

if [[ -f "$SERVER_KEY" || -f "$SERVER_CERT" ]] && [[ "$FORCE" -ne 1 ]]; then
    echo "Node certificate already exists in $NODE_DIR. Use --force to overwrite it." >&2
    exit 1
fi

tmp_config="$(mktemp)"
trap 'rm -f "$tmp_config"' EXIT

{
    echo "[req]"
    echo "distinguished_name = req_distinguished_name"
    echo "prompt = no"
    echo "req_extensions = req_ext"
    echo
    echo "[req_distinguished_name]"
    echo "CN = $NODE_NAME"
    echo
    echo "[req_ext]"
    echo "subjectAltName = @alt_names"
    echo "extendedKeyUsage = serverAuth"
    echo "keyUsage = digitalSignature, keyEncipherment"
    echo
    echo "[alt_names]"

    dns_index=1
    for dns in "${DNSES[@]}"; do
        echo "DNS.$dns_index = $dns"
        dns_index=$((dns_index + 1))
    done

    ip_index=1
    for ip in "${IPS[@]}"; do
        echo "IP.$ip_index = $ip"
        ip_index=$((ip_index + 1))
    done
} >"$tmp_config"

echo "Issuing certificate for node $NODE_NAME"
openssl genrsa -out "$SERVER_KEY" 4096
openssl req -new -key "$SERVER_KEY" -out "$SERVER_CSR" -config "$tmp_config"
openssl x509 \
    -req \
    -sha256 \
    -days "$NODE_DAYS" \
    -in "$SERVER_CSR" \
    -CA "$CA_CERT" \
    -CAkey "$CA_KEY" \
    -CAcreateserial \
    -CAserial "$CA_SERIAL" \
    -out "$SERVER_CERT" \
    -extfile "$tmp_config" \
    -extensions req_ext

cp "$CA_CERT" "$NODE_CA_CERT"
rm -f "$SERVER_CSR"
chmod 600 "$SERVER_KEY"

echo "Created:"
echo "  CA cert:     $CA_CERT"
echo "  Node cert:   $SERVER_CERT"
echo "  Node key:    $SERVER_KEY"
echo "  Bot CA cert: $NODE_CA_CERT"
