"""Decodificador bin/xtsw independente do servidor (para conferir capturas e respostas).

Uso:
  python xtsw.py schema.json <Tipo> <arquivo|base64>          decodifica um corpo
  python xtsw.py schema.json --reply <Tipo> <arquivo|base64>  decodifica uma resposta (envelope de 12 bytes)
  python xtsw.py schema.json --replay <captura.jsonl> [http://127.0.0.1:8081]
      reenvia as requisicoes Forza da captura ao servidor e decodifica requisicao e resposta
"""
import base64
import json
import os
import struct
import sys
import urllib.request


class R:
    def __init__(self, b):
        self.b, self.p = b, 0

    def take(self, n):
        if self.p + n > len(self.b):
            raise ValueError(f"faltam bytes: {n} em {self.p}, restam {len(self.b) - self.p}")
        s = self.b[self.p:self.p + n]
        self.p += n
        return s

    def u(self, fmt):
        return struct.unpack(">" + fmt, self.take(struct.calcsize(fmt)))[0]


SCALAR = {"u8": "B", "bool": "?", "i16": "h", "u16": "H", "i32": "i", "u32": "I",
          "i64": "q", "u64": "Q", "f32": "f", "f64": "d"}


def read(types, kind, ty, elem, r):
    if kind in SCALAR:
        return r.u(SCALAR[kind])
    if kind in ("string", "datetime"):
        n = r.u("I"); assert n <= 0x2000, n
        return r.take(n).decode("utf-8", "replace")
    if kind == "blob":
        n = r.u("I"); assert n <= 0x80000, n
        return r.take(n).hex()
    if kind == "guid":
        h = r.take(16).hex()
        return f"{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}"
    if kind == "struct":
        return read_struct(types, ty, r)
    if kind == "array":
        n = r.u("I"); assert n <= 0x2000, n
        return [read(types, elem["kind"], elem.get("type"), None, r) for _ in range(n)]
    raise ValueError(f"tipo {kind} nao suportado")


def read_struct(types, ty, r):
    out = {}
    for f in types[ty]["fields"]:
        out[f["name"]] = read(types, f["kind"], f.get("type"), f.get("elem"), r)
    return out


def write(types, kind, ty, elem, v, out):
    if kind in SCALAR:
        out += struct.pack(">" + SCALAR[kind], v if v is not None else 0)
    elif kind in ("string", "datetime"):
        b = (v or ("2000-01-01T00:00:00Z" if kind == "datetime" else "")).encode()
        out += struct.pack(">I", len(b)) + b
    elif kind == "blob":
        b = bytes.fromhex(v or "")
        out += struct.pack(">I", len(b)) + b
    elif kind == "guid":
        out += bytes.fromhex((v or "0" * 32).replace("-", ""))
    elif kind == "struct":
        write_struct(types, ty, v or {}, out)
    elif kind == "array":
        items = v or []
        out += struct.pack(">I", len(items))
        for it in items:
            write(types, elem["kind"], elem.get("type"), None, it, out)
    else:
        raise ValueError(kind)


def write_struct(types, ty, obj, out):
    for f in types[ty]["fields"]:
        write(types, f["kind"], f.get("type"), f.get("elem"), obj.get(f["name"]), out)


def encode(types, ty, obj):
    """Encodes a JSON object as a _Parameters body (missing fields = zero), like the game."""
    out = bytearray()
    write_struct(types, ty, obj, out)
    return bytes(out)


def decode(types, ty, data):
    r = R(data)
    v = read_struct(types, ty, r)
    if r.p != len(data):
        raise ValueError(f"{len(data) - r.p} bytes sobrando")
    return v


def decode_reply(types, ty, data):
    status, code, n = struct.unpack(">III", data[:12])
    if status != 0:
        return {"status": status, "hresult": hex(0x80510000 | code), "message": data[12:12 + n].decode("utf-8", "replace")}
    return {"status": 0, "results": decode(types, ty, data[12:]) if ty else None}


def load(arg):
    if os.path.exists(arg):
        return open(arg, "rb").read()
    return base64.b64decode(arg)


def replay(schema, capture, server):
    for line in open(capture, encoding="utf-8"):
        rec = json.loads(line)
        hdrs = rec.get("request_headers")
        if isinstance(hdrs, str):  # captura do hook: bloco de texto
            hdrs = dict(l.split(": ", 1) for l in hdrs.split("\r\n") if ": " in l)
        hdrs = {k.lower(): (v[0] if isinstance(v, list) else v) for k, v in hdrs.items()}
        cls = hdrs.get("x-classname")
        if not cls:
            continue
        meth = hdrs["x-method"]
        key = cls.split(",")[0]
        m = schema["services"].get(key, {}).get("methods", {}).get(meth)
        body = base64.b64decode(rec["request_body_b64"])
        url = rec.get("original_url") or rec.get("url")
        host_path = url.split("://", 1)[1]
        headers = {"Content-Type": "bin/xtsw", "X-ClassName": cls, "X-Method": meth, "X-Fh2-Original-Url": url}
        if hdrs.get("x-userid"):
            headers["X-UserId"] = hdrs["x-userid"]
        req = urllib.request.Request(f"{server}/{host_path}", data=body, method="POST", headers=headers)
        try:
            with urllib.request.urlopen(req) as resp:
                data = resp.read()
                st = resp.status
        except urllib.error.HTTPError as e:
            print(f"== {key}.{meth} -> HTTP {e.code} (sem handler)")
            continue
        print(f"== {key}.{meth} -> HTTP {st}, {len(data)} bytes")
        if m:
            if m["params"]:
                print(" params:", json.dumps(decode(schema["types"], m["params"], body))[:300])
            print(" reply: ", json.dumps(decode_reply(schema["types"], m["results"], data))[:2000])


def main():
    schema = json.load(open(sys.argv[1], encoding="utf-8"))
    if sys.argv[2] == "--replay":
        replay(schema, sys.argv[3], sys.argv[4] if len(sys.argv) > 4 else "http://127.0.0.1:8081")
    elif sys.argv[2] == "--reply":
        print(json.dumps(decode_reply(schema["types"], sys.argv[3], load(sys.argv[4])), indent=1))
    else:
        print(json.dumps(decode(schema["types"], sys.argv[2], load(sys.argv[3])), indent=1))


if __name__ == "__main__":
    main()
