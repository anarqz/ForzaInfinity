import rq, json, sys
targets = {
 "0x140254078":"Not a game folder: no Mount\\AppxManifest.xml",
 "0x1402541e0":"Incomplete or another version: Mount\\hash.manifest",
 "0x140253f60":"The pinned game manifest does not name its executable, entry point...",
 "0x140253e60":"EntryPoint",
 "0x14024ebc8":"Mapped ",
 "0x140255060":"not a PE32+ AMD64 image",
 "0x140255030":"cannot be mapped",
 "0x140255078":"invalid relocation block",
 "0x140255098":"unsupported relocation",
 "0x140254f60":"cannot protect a section",
 "0x140254f18":"RVA outside the mapping",
 "0x14024fc68":"Cannot allocate local import thunks",
 "0x14024fc90":"Cannot protect import thunks",
 "0x140254e58":"GetProcAddress unsupported export=",
 "0x140254f48":"forwarded export",
 "0x140254d20":"More images mapped by hand than the registry holds",
 "0x140255660":"EmbeddedXvd/Windows/System32/xg_x.dll",
 "0x140255720":"Unexpected XG ordinal dependency",
 "0x140255208":"Private desktop VC11 export unavailable",
 "0x140263898":"XSAPI image",
 "0x140257f78":"Unexpected game image layout",
 "0x140257fc8":"Cannot load the native static-TLS helper",
 "0x140257ff8":"Unexpected static TLS template or callbacks",
 "0x140258028":"Windows did not initialize the static TLS helper",
 "0x14024df80":"A scripted run needs --game=DIR",
 "0x1402680d0":"not a PE32+ image (second parser)",
}
result = {}
for addr, label in targets.items():
    try:
        refs = rq.call("xrefs", {"address": addr})
    except Exception as e:
        refs = []; print("xref err", addr, e)
    procs = []
    for ref in refs if isinstance(refs, list) else []:
        p = rq.call("resolve_containing_procedure", {"address": ref})
        if p.get("found"):
            proc = p["procedure"]
            procs.append({"ref": ref, "proc": proc.get("address") if isinstance(proc, dict) else proc,
                          "name": proc.get("name") if isinstance(proc, dict) else None})
        else:
            procs.append({"ref": ref, "proc": None})
    result[addr] = {"label": label, "uses": procs}
    print(f"{addr}  {label[:60]:60} -> " + ", ".join(f"{u['proc']}({u.get('name')})" for u in procs))
json.dump(result, open("/home/claude/work/out/string_procs.json","w"), indent=1)
