#!/bin/bash
# Renders og/card.html to public/og.png (1200x630, the social sharing card) with headless
# Firefox. The page is served locally with a deliberately slow 1-pixel image, so the
# screenshot (taken on the load event) waits until the web font has arrived.
#   og/render.sh
set -euo pipefail
cd "$(dirname "$0")"
WORK=$(mktemp -d)
trap 'kill "${SERVER:-0}" 2>/dev/null || true; rm -rf "$WORK"' EXIT
cp card.html "$WORK/index.html"
sed -i 's|</body>|<img src="/slow" width="1" height="1" style="position:absolute;opacity:0" /></body>|' "$WORK/index.html"
cat > "$WORK/serve.py" <<'EOF'
import http.server, os, sys, time
os.chdir(sys.argv[1])
class Handler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/slow'):
            time.sleep(4)
            self.send_response(200); self.send_header('Content-Type', 'image/gif'); self.end_headers()
            self.wfile.write(bytes.fromhex('47494638396101000100800000000000ffffff21f90401000000002c00000000010001000002024401003b'))
        else:
            super().do_GET()
    def log_message(self, *args):
        pass
http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[2])), Handler).serve_forever()
EOF
PORT=18765
python3 -I "$WORK/serve.py" "$WORK" "$PORT" &
SERVER=$!
sleep 1
mkdir -p "$WORK/profile"
firefox --headless --no-remote --profile "$WORK/profile" --window-size=1200,630 \
  --screenshot "$(pwd)/../public/og.png" "http://127.0.0.1:$PORT/index.html" >/dev/null 2>&1
echo "public/og.png: $(identify -format '%wx%h' ../public/og.png 2>/dev/null || echo written)"
