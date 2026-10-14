#!/bin/bash
# terraform with the credentials of whoever runs it: the gcloud login (no key files) and the
# Cloudflare credentials in ../.env (gitignored): CLOUDFLARE_API_TOKEN=..., or the global key
# as CLOUDFLARE_GLOBAL_API_TOKEN=... with CLOUDFLARE_EMAIL=...
#   ./tf.sh init | plan | apply | output
set -euo pipefail
cd "$(dirname "$0")"
ENV_FILE="${INFINITY_ENV:-../.env}"
if [ ! -r "$ENV_FILE" ]; then
  echo "missing $ENV_FILE (CLOUDFLARE_API_TOKEN=...)" >&2
  exit 1
fi
set -a
. "$ENV_FILE"
set +a
if [ -z "${CLOUDFLARE_API_TOKEN:-}" ] && [ -n "${CLOUDFLARE_GLOBAL_API_TOKEN:-}" ]; then
  export CLOUDFLARE_API_KEY="$CLOUDFLARE_GLOBAL_API_TOKEN"
  : "${CLOUDFLARE_EMAIL:?the global key needs CLOUDFLARE_EMAIL}"
fi
unset CLOUDFLARE_GLOBAL_API_TOKEN
GOOGLE_OAUTH_ACCESS_TOKEN=$(gcloud auth print-access-token)
export GOOGLE_OAUTH_ACCESS_TOKEN
exec terraform "$@"
