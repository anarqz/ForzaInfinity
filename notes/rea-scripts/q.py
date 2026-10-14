# Usage: q.py TOOL 'JSON_ARGS' [OUTFILE]  -> prints result (and saves it)
import socket, json, sys
tool = sys.argv[1]; args = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
s = socket.socket(socket.AF_UNIX); s.connect("/home/claude/work/rea.sock")
s.sendall((json.dumps({"tool": tool, "args": args}) + "\n").encode())
buf = b""
while not buf.endswith(b"\n"):
    chunk = s.recv(1 << 20)
    if not chunk: break
    buf += chunk
out = json.loads(buf)
if len(sys.argv) > 3:
    open(sys.argv[3], "w").write(out["text"])
print(("ERROR: " if out["isError"] else "") + out["text"])
