"""Mostra os tipos de entrada/saida de metodos. Uso: python show_methods.py schema.json [--params] Classe.Metodo ..."""
import json
import sys


def show(types, name, ind=0, seen=()):
    for f in types[name]["fields"]:
        sub = f.get("type") or (f.get("elem") or {}).get("type")
        kind = f["kind"] + ("<" + f["elem"]["kind"] + ">" if f["kind"] == "array" else "")
        cap = {k: f[k] for k in ("max_chars", "max_count", "max_len") if f.get(k)}
        print(" " * ind + f"{f['name']}: {kind}{':' + sub if sub else ''} {cap if cap else ''}")
        if sub and sub not in seen and ind < 8:
            show(types, sub, ind + 2, seen + (name,))


def main():
    d = json.load(open(sys.argv[1], encoding="utf-8"))
    args = sys.argv[2:]
    params = "--params" in args
    for m in [a for a in args if a != "--params"]:
        cls, _, meth = m.rpartition(".")
        md = d["services"][cls]["methods"][meth]
        for slot in (("params", "results") if params else ("results",)):
            print(f"===== {m} {slot} -> {md[slot]}")
            if md[slot]:
                show(d["types"], md[slot], 2)


if __name__ == "__main__":
    main()
