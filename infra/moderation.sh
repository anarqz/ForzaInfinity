#!/bin/bash
# Moderation of the public server: the accounts players reported (PLAYER ACTIONS > REPORT PLAYER).
# It runs `fh2-server moderation` on the VM over IAP SSH, as the server's user, against its
# database (no HTTP route, no key in this repository: your own gcloud login with SSH/IAP access
# to the project lets you in, as for deploy.sh). Every command prints JSON.
#   ./moderation.sh list                 flagged accounts (5+ reports), oldest flag first
#   ./moderation.sh show XUID            reports, records, profile, garage, gates, moderation log
#   ./moderation.sh clear XUID "why"     nothing wrong: the flag and the reports go
#   ./moderation.sh reset XUID "why"     cheating: progress back to a new profile's, liveries kept
#   ./moderation.sh files                shared files (liveries, vinyls, tunes) under review after 3 reports
#   ./moderation.sh file ID              one file: details and reports
#   ./moderation.sh preview ID OUT       its preview image, saved to OUT (to look at it)
#   ./moderation.sh file-keep ID "why"   nothing wrong: back on the storefront
#   ./moderation.sh file-delete ID "why" offensive, stolen or broken: gone for good
# clear, reset, file-keep and file-delete need the why (it goes to moderation_log). docs/SERVER.md.
set -euo pipefail
cd "$(dirname "$0")"
PROJECT=${PROJECT:-forza-infinity-online}
ZONE=${ZONE:-us-east1-b}
VM=infinity
MOD="sudo -u infinity /opt/infinity/bin/fh2-server moderation --root /opt/infinity --db /opt/infinity/data/fh2online.sqlite"

remote() {
    gcloud compute ssh "$VM" --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap --quiet --command "$1" 2>/dev/null
}

usage() {
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

xuid() { [[ "${1:-}" =~ ^[0-9]{1,20}$ ]] || { echo "XUID: digits only" >&2; exit 2; }; }

number() { [[ "${1:-}" =~ ^[0-9]{1,12}$ ]] || { echo "ID: digits only" >&2; exit 2; }; }

case "${1:-}" in
    list) remote "$MOD list" ;;
    files) remote "$MOD files" ;;
    file) number "${2:-}"; remote "$MOD file $2" ;;
    preview)
        number "${2:-}"
        [ -n "${3:-}" ] || { echo "usage: ./moderation.sh preview ID OUT" >&2; exit 2; }
        remote "$MOD file $2 --preview-only" | base64 -d > "$3"
        echo "$3: $(wc -c < "$3") bytes"
        ;;
    file-keep | file-delete)
        number "${2:-}"
        [ -n "${3:-}" ] || { echo "$1 needs the why: ./moderation.sh $1 ID \"why\"" >&2; exit 2; }
        remote "$MOD $1 $2 --note-base64 $(printf '%s' "$3" | base64 -w0)"
        ;;
    show) xuid "${2:-}"; remote "$MOD show $2" ;;
    clear | reset)
        xuid "${2:-}"
        [ -n "${3:-}" ] || { echo "$1 needs the why: ./moderation.sh $1 XUID \"why\"" >&2; exit 2; }
        # base64: no quoting of the note through ssh and the remote shell
        remote "$MOD $1 $2 --note-base64 $(printf '%s' "$3" | base64 -w0)"
        ;;
    *) usage ;;
esac
