"""Mostra a arvore de um tipo do schema.json. Uso: python show_type.py schema.json <Tipo> [...]"""
import json
import sys


def show(types, name, ind=0, seen=()):
    t = types.get(name)
    if t is None:
        print(" " * ind + f"?? {name}")
        return
    print(" " * ind + f"{name} (size {t['size']:#x})")
    for f in t["fields"]:
        extra = {k: v for k, v in f.items() if k not in ("name", "kind", "type", "elem", "offset", "size")}
        sub = f.get("type") or (f.get("elem") or {}).get("type")
        desc = f["kind"]
        if f["kind"] == "array":
            desc += f"<{f['elem']['kind']}{':' + sub if sub else ''}>"
        elif sub:
            desc += f":{sub}"
        print(" " * ind + f"  {f['name']:32} {desc} {extra if extra else ''}")
        if sub and sub not in seen and sub != name:
            show(types, sub, ind + 4, seen + (name,))


if __name__ == "__main__":
    d = json.load(open(sys.argv[1], encoding="utf-8"))
    for n in sys.argv[2:]:
        show(d["types"], n)
