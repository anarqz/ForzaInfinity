#!/bin/bash
# Admin news for the public server: a message in every player's in-game Message Center.
# It runs `fh2-server news` on the VM over IAP SSH, as the server's user, against its database:
# there is no HTTP route for news, and no key in this repository. What lets you in is your own
# gcloud login with SSH/IAP access to the project (the same as deploy.sh).
#   ./news.sh send "Text" [--to Name#12345]   shows the text and the player count, asks, sends
#   ./news.sh send -f message.txt [--to ...]   the text from a file (line breaks kept)
#   ./news.sh list                             past sends
#   ./news.sh delete ID                        takes a send back from every Message Center
# Line breaks: real ones, or \n typed. At most 512 characters.
set -euo pipefail
cd "$(dirname "$0")"
PROJECT=${PROJECT:-forza-infinity-online}
ZONE=${ZONE:-us-east1-b}
VM=infinity
NEWS="sudo -u infinity /opt/infinity/bin/fh2-server news --db /opt/infinity/data/fh2online.sqlite"

remote() {
    gcloud compute ssh "$VM" --project "$PROJECT" --zone "$ZONE" --tunnel-through-iap --quiet --command "$1" 2>/dev/null
}

case "${1:-}" in
    send)
        shift
        TEXT=""; TO=""
        while [ $# -gt 0 ]; do
            case "$1" in
                -f) TEXT="$(cat "$2")"; shift 2 ;;
                --to) TO="$2"; shift 2 ;;
                *) TEXT="$1"; shift ;;
            esac
        done
        [ -n "$TEXT" ] || { echo "usage: ./news.sh send \"Text\" [--to Name#12345]" >&2; exit 2; }
        if [ -n "$TO" ] && ! [[ "$TO" =~ ^[A-Za-z][A-Za-z0-9]{0,8}#[0-9]{5}$ ]]; then
            echo "--to takes Name#12345" >&2; exit 2
        fi
        # base64 on the command line: no quoting of the text through ssh and the remote shell
        B64="$(printf '%s' "$TEXT" | base64 -w0)"
        ARGS="send --text-base64 $B64${TO:+ --to $TO}"
        remote "$NEWS $ARGS --dry-run"
        read -r -p "Send it? [y/N] " answer </dev/tty
        [ "$answer" = y ] || [ "$answer" = Y ] || { echo "not sent"; exit 1; }
        remote "$NEWS $ARGS"
        ;;
    list)
        remote "$NEWS list"
        ;;
    delete)
        [[ "${2:-}" =~ ^[0-9]+$ ]] || { echo "usage: ./news.sh delete ID" >&2; exit 2; }
        remote "$NEWS delete $2"
        ;;
    *)
        sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
