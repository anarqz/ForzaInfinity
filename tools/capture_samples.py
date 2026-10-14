"""Uma amostra decodificada de cada metodo Forza numa captura do servidor.
Uso: python capture_samples.py captura.jsonl [max_chars] [Metodo ...]"""
import json
import sys

path = sys.argv[1]
limit = int(sys.argv[2]) if len(sys.argv) > 2 else 1500
only = set(sys.argv[3:])
seen = set()
for line in open(path, encoding="utf-8"):
    r = json.loads(line)
    m = r.get("forza_method")
    if not m:
        continue
    key = f"{r['forza_class']}.{m}"
    if key in seen or (only and m not in only):
        continue
    seen.add(key)
    text = json.dumps(r.get("request_xtsw"), ensure_ascii=False)
    body = len(r.get("request_body_b64") or "") * 3 // 4
    print(f"===== {key} ({body} bytes)\n{text[:limit]}{' ...' if len(text) > limit else ''}")
