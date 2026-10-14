import socket, json
def call(tool, args=None):
    s = socket.socket(socket.AF_UNIX); s.connect("/home/claude/work/rea.sock")
    s.sendall((json.dumps({"tool": tool, "args": args or {}}) + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        c = s.recv(1 << 20)
        if not c: break
        buf += c
    out = json.loads(buf)
    if out["isError"]: raise RuntimeError(out["text"][:1500])
    try: d = json.loads(out["text"])
    except Exception: return out["text"]
    return d.get("normalized_result", d) if isinstance(d, dict) else d
