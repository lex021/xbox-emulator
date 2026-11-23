import json, os, math, sys
from xml.sax.saxutils import escape

here = os.path.dirname(os.path.abspath(__file__))
root = os.path.normpath(os.path.join(here, "..", ".."))
dat = os.path.join(here, "data")
img = os.path.join(root, "docs", "img")


def om_name(m):
    return m["name"] if isinstance(m, dict) else m


def om_state(m):
    return m.get("state", "not reached") if isinstance(m, dict) else "not reached"


CONFIG = sys.argv[sys.argv.index("--config") + 1] if "--config" in sys.argv else os.path.join(here, "progress.json")
cfg = json.load(open(CONFIG, encoding="utf-8"))
_emu = {k.lower(): {**v, "functions": dict(v.get("functions", {})), "notes": dict(v.get("notes", {}))} for k, v in cfg["emulator"].items()}
_xp = json.load(open(os.path.join(dat, "emulator_exports.json"), encoding="utf-8")) if os.path.isfile(os.path.join(dat, "emulator_exports.json")) else {}
_xpl = {k.lower() for k in _xp}
if os.path.isfile(os.path.join(dat, "emulator_cfg_checks.json")):
    for _dll, _c in json.load(open(os.path.join(dat, "emulator_cfg_checks.json"), encoding="utf-8")).items():
        _e = _emu.get(_dll.lower())
        if _e is None or _dll.lower() in _xpl:
            continue
        for _f in _c.get("not_in_built_dll", []):
            _e["functions"].pop(_f, None)
            _e["notes"][_f] = f"not exported by the built {_c['built_dll']}" + (f"; {_e['notes'][_f]}" if _e["notes"].get(_f) else "")
for _dll, _v in _xp.items():
    _e = _emu.setdefault(_dll.lower(), {})
    _e["file"] = _v["file"]
    _e["functions"] = {f: s for f, s in _v["functions"].items() if s != "missing"}
    _e["notes"] = {**_e.get("notes", {}), **_v["notes"]}
    if _v.get("builds"):
        _e["builds"] = {b: {"table": t["table"], "functions": {f: s for f, s in t["functions"].items() if s != "missing"}, "notes": t["notes"]}
                        for b, t in _v["builds"].items()}
        _e["known_builds"] = _v["known_builds"]
cfg["emulator"] = _emu


def variant(e, build):
    vs = sorted(int(b) for b in e.get("builds", {}))
    kb = e.get("known_builds", vs)
    if build is None:
        return None
    for i, b in enumerate(vs):
        upper = next((k for k in kb if k > b), b + 1)
        if build < upper and (build >= b or i == 0):
            return str(b)
    return None


def emu_table(e, build):
    v = variant(e, build)
    return e["builds"][v] if v else e
EVIDENCE_OVERRIDE = None


def evidence(name):
    if EVIDENCE_OVERRIDE is not None:
        return EVIDENCE_OVERRIDE.get(name)
    p = os.path.join(dat, "evidence", name)
    return json.load(open(p, encoding="utf-8")) if os.path.isfile(p) else None


def ev_modules(name):
    e = evidence(name) or {}
    return {k.lower(): v for k, v in e.get("modules", {}).items()}


def module_proven(name, mod):
    m = ev_modules(name).get(mod.lower())
    return bool(m and m.get("loaded") is True and isinstance(m.get("unresolved"), list) and not m["unresolved"])


def function_proven(name, mod, f):
    m = ev_modules(name).get(mod.lower())
    if not m or m.get("loaded") is not True:
        return False
    if f == "(delay-loaded)":
        return module_proven(name, mod)
    return f in m.get("resolved", []) and f not in m.get("unresolved", [])


def oshost_hooks():
    e = evidence("oshost.json") or {}
    return {k.lower(): set(v) for k, v in e.get("hooks", {}).items()}, e.get("file")


IMPORT_EVIDENCE = ["media.json", "gameinput_ext.json", "chatprobe.json"]
GREEN, AMBER, GREY, BG = "#2ea043", "#d29922", "#6e7681", "#0d1117"
COL = {"done": GREEN, "partial": AMBER, "none": GREY}
XCOL = {"implemented": "#2ea043", "forwarded": "#388bfd", "hooked": "#a371f7", "pc": "#2f8f83",
        "title": "#56b6c2", "pending": "#e3b341", "stub": "#d29922", "unsupported": "#da3633", "unverified": "#3d444d", "missing": "#6e7681"}
XORDER = ["implemented", "forwarded", "hooked", "pc", "title", "pending", "stub", "unsupported", "unverified", "missing"]
COVERED = {"implemented", "forwarded", "hooked", "pc", "title"}
PC_SUPPLEMENT = {"kernel32.dll", "advapi32.dll", "ntdll.dll", "kernelbase.dll", "shcore.dll", "ole32.dll"}
LABEL = {"implemented": "implemented in emulator", "forwarded": "forwarded by emulator to PC",
         "hooked": "PC system DLL, hooked by emulator", "pc": "PC system DLL (name match)",
         "title": "shipped with the title", "pending": "implemented, awaiting verification", "stub": "emulator stub", "unsupported": "my PC doesn't support it", "unverified": "unverified",
         "missing": "missing"}
TOMBOX = "https://github.com/drycheetah84/TomBox/blob/master/"


def jload(name):
    return json.load(open(os.path.join(dat, name), encoding="utf-8"))


_CACHE = {}


def cached(key):
    if key not in _CACHE:
        _CACHE[key] = {k: v.get("build") for k, v in jload("title_imports.json").items()} if key == "builds" else jload("console_exports.json")
    return _CACHE[key]


def ranges(s):
    out = set()
    for part in filter(None, s.split(",")):
        a, _, b = part.partition("-")
        out.update(range(int(a), int(b or a) + 1))
    return out


def imports():
    exe = jload("fortnite_exe.json")
    ov = cfg["import_overrides"]
    groups = {}
    mods = [(m["module"], m["functions"]) for m in exe["imports"]] + [(m, ["(delay-loaded)"]) for m in exe.get("delay_imports", [])]
    for mod, funcs in mods:
        low = mod.lower()
        if low.startswith("api-ms-win-crt") or low.startswith("vcruntime") or low.startswith("msvcp"):
            g = "C/C++ runtime"
        elif low.startswith("api-ms-") or low.startswith("ext-ms-"):
            g = "Windows API sets"
        else:
            g = mod.rsplit(".", 1)[0]
        for f in funcs:
            st = ov.get(f"{mod}!{f}", ov.get(f, ov.get(mod, "done")))
            if st != "done" and any(function_proven(e, mod, f) for e in IMPORT_EVIDENCE):
                st = "done"
            groups.setdefault(g, []).append(st)
    return groups


def d3d12x():
    groups = {}
    for name in ("Device", "CommandList"):
        d = cfg["d3d12x"][name]
        done, part = ranges(d["done"]), ranges(d["partial"])
        groups[name] = ["done" if i in done else "partial" if i in part else "none" for i in range(d["total"])]
    return groups


def instructions():
    used = {}
    for line in open(os.path.join(dat, "mnemonics.txt"), encoding="utf-8"):
        n, u, ok = line.split()
        used[n] = (int(u), int(ok))
    verified = set(cfg.get("rdna2_verified", {}).get("mnemonics", []))
    pending = set(cfg.get("rdna2_pending", {}).get("mnemonics", [])) - verified
    groups, seen = {}, set()
    for g, names in jload("mnem_groups.json"):
        for n in names:
            seen.add(n)
            u, ok = used.get(n, (0, 0))
            if n in verified and u == ok:
                groups.setdefault(g, []).append("done")
                continue
            if n in pending:
                groups.setdefault(g, []).append("partial")
                continue
            groups.setdefault(g, []).append("done" if u and u == ok else "partial" if u else "none")
    for n, (u, ok) in used.items():
        if n not in seen:
            g = "EXP" if n == "exp" else "DS" if n.startswith("ds_") else "OTHER"
            groups.setdefault(g, []).append("done" if u == ok else "partial")
    return groups


def squarify(items, x, y, w, h):
    out, items = [], list(items)
    total = sum(a for _, a in items)
    scale = w * h / total
    items = [(k, a * scale) for k, a in items]

    def worst(row, side):
        s = sum(a for _, a in row)
        return max(max(side * side * a / (s * s), s * s / (side * side * a)) for _, a in row)
    while items:
        side = min(w, h)
        row = [items.pop(0)]
        while items and worst(row + [items[0]], side) <= worst(row, side):
            row.append(items.pop(0))
        s = sum(a for _, a in row)
        if w >= h:
            cw, cy = s / h, y
            for k, a in row:
                out.append((k, x, cy, cw, a / cw))
                cy += a / cw
            x += cw
            w -= cw
        else:
            ch, cx = s / w, x
            for k, a in row:
                out.append((k, cx, y, a / ch, ch))
                cx += a / ch
            y += ch
            h -= ch
    return out


def text(x, y, s, size=12, fill="#ffffff"):
    return (f'<text x="{x:.2f}" y="{y:.2f}" font-family="sans-serif" font-size="{size}" fill="{fill}" stroke="{BG}" '
            f'stroke-width="3" paint-order="stroke">{escape(s)}</text>')


def section(o, y, title, groups, height):
    n = sum(len(v) for v in groups.values())
    done = sum(v.count("done") for v in groups.values())
    o.append(text(4, y + 21, f"{title}: {100 * done / n:.2f}% ({done}/{n})", 16))
    y += 30
    order = sorted(groups.items(), key=lambda kv: -len(kv[1]))
    for key, cx, cy, cw, ch in squarify([(k, len(v)) for k, v in order], 0, y, 1000, height):
        st = sorted(groups[key], key=["done", "partial", "none"].index)
        cnt = len(st)
        ix, iy, iw, ih = cx + 1, cy + 1, cw - 2, ch - 2
        cols = max(1, min(cnt, math.ceil(math.sqrt(cnt * iw / ih))))
        rows = math.ceil(cnt / cols)
        sw, sh = iw / cols, ih / rows
        o.append(f'<g><title>{escape(key)}: {st.count("done")}/{cnt} ({100 * st.count("done") // cnt}%)</title>')
        for i, s in enumerate(st):
            o.append(f'<rect x="{ix + (i % cols) * sw:.2f}" y="{iy + (i // cols) * sh:.2f}" width="{sw:.2f}" height="{sh:.2f}" '
                     f'fill="{COL[s]}" stroke="{BG}" stroke-width="0.5"/>')
        o.append(f'<rect x="{cx:.2f}" y="{cy:.2f}" width="{cw:.2f}" height="{ch:.2f}" fill="none" stroke="{BG}" stroke-width="2"/>')
        label = key[: max(0, int((cw - 6) / 7))]
        if label and ch >= 18:
            o.append(text(cx + 5, cy + 16, label))
        o.append("</g>")
    return y + height + 10, done, n


def lighten(c, t=0.45):
    r, g, b = (int(c[i:i + 2], 16) for i in (1, 3, 5))
    return "#%02x%02x%02x" % tuple(int(v + (255 - v) * t) for v in (r, g, b))


def title_import_index():
    ti = jload("title_imports.json")
    direct, by_func = {}, {}
    for title, t in ti.items():
        for mods in t["binaries"].values():
            for m in mods:
                low = m["module"].lower()
                hosts = m.get("hosts") or [None] * len(m["functions"])
                names = m.get("names") or [None] * len(m["functions"])
                for f, h, nm in zip(m["functions"], hosts, names):
                    if h:
                        direct.setdefault((h, f), set()).add(title)
                    elif low.startswith(("api-ms-", "ext-ms-")):
                        by_func.setdefault(f, set()).add(title)
                    else:
                        direct.setdefault((low, f), set()).add(title)
                        if nm:
                            direct.setdefault((low, nm), set()).add(title)
        for libs in t.get("dynamic", {}).values():
            for lib, fs in libs.items():
                for f in fs:
                    direct.setdefault((lib.lower(), f), set()).add(title)
    return ti, direct, by_func


def emu_row_status(e, n, pres, builds):
    tabs = [emu_table(e, builds.get(t)) for t in pres] or [e]
    return max((t["functions"].get(n, "missing") for t in tabs), key=XORDER.index)


HOST_PRIORITY = ["kernelbase.dll", "ucrtbase.dll", "ntdll.dll", "combase.dll", "msvcp_win.dll", "rpcrt4.dll",
                 "sechost.dll", "minuserx.dll", "oleaut32.dll"]


def console(full=False, twins=True):
    con = jload("console_exports.json")
    ti, direct, by_func = title_import_index()
    builds = {k: v.get("build") for k, v in ti.items()}
    emu = {k.lower(): v for k, v in cfg["emulator"].items()}
    hooked = {k: set(v) for k, v in cfg["hooked"].items() if k != "file"}
    oshook, oshook_file = oshost_hooks()
    host = {}
    for lib in sorted(con, key=lambda l: (HOST_PRIORITY.index(l.lower()) if l.lower() in HOST_PRIORITY else 99, l.lower())):
        for n, has, fwd, pres in con[lib]["exports"]:
            if fwd is None and n in by_func and n not in host:
                host[n] = lib.lower()
    libs = {}
    cpend = {k.lower(): set(v) for k, v in cfg.get("console_pending", {}).items()}
    for lib, v in con.items():
        low = lib.lower()
        rows, notes = [], {}
        twin = set(v.get("twin_has", ())) if twins and not v["pc_dll"] and low not in emu else set()
        tw = " / ".join(v.get("pc_twin", ()))
        for n, has, fwd, pres in v["exports"]:
            if not full and has != 0:
                continue
            users = set(direct.get((low, n), set()))
            if host.get(n) == low:
                users |= by_func.get(n, set())
            if low in emu and (has == 0 or any(n in t["functions"] for t in [emu[low], *emu[low].get("builds", {}).values()]) or low not in PC_SUPPLEMENT):
                st = emu_row_status(emu[low], n, pres, builds)
            elif has == 1:
                st = "hooked" if n in hooked.get(low, ()) else "pc"
            elif has == 2:
                st = "unverified"
            elif users and n in twin:
                hk = n in oshook.get(low, ()) or any(n in oshook.get(t, ()) for t in v.get("pc_twin", ()))
                st = "hooked" if hk else "pc"
                notes[n] = (f"PC twin {tw} exports {n}; hooked by the emulator OS host ({oshook_file})" if hk
                            else f"name match on the PC twin {tw}; the console {lib} is not emulated, the title's API-set import resolves to the PC DLL")
            else:
                st = "missing"
            if n in cpend.get(low, ()) and st in COVERED:
                st = "pending"
            rows.append((n, st, frozenset(users), tuple(pres)))
        if not rows:
            continue
        libs[lib] = {"rows": rows, "pc_dll": v["pc_dll"], "dir": v["dir"], "versions": v["versions"],
                     "pc_twin": v.get("pc_twin", []), "notes": notes, "emulated": low in emu}
    for g, v in cfg["gdk_linked"].items():
        tt = v.get("titles", {})
        st = v.get("status", {})
        libs[g] = {"rows": [(n, st.get(n, "implemented"), frozenset(tt.get(n, ["Fortnite"])), tuple(sorted(tt.get(n, ["Fortnite"])))) for n in v["functions"]],
                   "pc_dll": False, "dir": v.get("dir", "Fortnite EXE (statically linked)"), "versions": {}}
    return libs


def not_targeted(libs):
    emu = {k.lower() for k in cfg["emulator"]}
    nt, tg = [], []
    for lib, v in libs.items():
        (tg if lib.lower() in emu or any(r[2] for r in v["rows"]) else nt).append(lib)
    return nt, tg


def console_section(o, y, libs, height):
    allrows = [r for v in libs.values() for r in v["rows"]]
    n = len(allrows)
    cov = sum(1 for r in allrows if r[1] in COVERED)
    o.append(text(4, y + 21, f"Xbox console libraries: {100 * cov / n:.2f}% ({cov}/{n})", 16))
    y += 30
    lx = 4
    present = {r[1] for r in allrows}
    for s in XORDER:
        if s not in present:
            continue
        o.append(f'<rect x="{lx}" y="{y + 2}" width="10" height="10" fill="{XCOL[s]}"/>')
        nm = "my PC doesn't support it" if s == "unsupported" else s
        o.append(text(lx + 14, y + 11, nm, 11))
        lx += 14 + 7 * len(nm) + 14
    o.append(f'<rect x="{lx}" y="{y + 2}" width="10" height="10" fill="{lighten(XCOL["missing"])}"/>')
    o.append(text(lx + 14, y + 11, "lighter = imported by an analysed title", 11))
    y += 18
    order = sorted(libs.items(), key=lambda kv: -len(kv[1]["rows"]))
    for key, cx, cy, cw, ch in squarify([(k, len(v["rows"])) for k, v in order], 0, y, 1000, height):
        rows = sorted(libs[key]["rows"], key=lambda r: (XORDER.index(r[1]), not r[2]))
        cnt = len(rows)
        ix, iy, iw, ih = cx + 1, cy + 1, cw - 2, ch - 2
        cols = max(1, min(cnt, math.ceil(math.sqrt(cnt * iw / ih))))
        nrows = math.ceil(cnt / cols)
        sw, sh = iw / cols, ih / nrows
        c = sum(1 for r in rows if r[1] in COVERED)
        per = {}
        for r in rows:
            for t in r[2]:
                per[t] = per.get(t, 0) + 1
        used = ", ".join(f"{t} imports {k}" for t, k in sorted(per.items())) or "not imported by an analysed title"
        o.append(f'<g><title>{escape(key)}: {c}/{cnt} covered; {escape(used)}</title>')
        i = 0
        while i < cnt:
            row, col = divmod(i, cols)
            fill = lighten(XCOL[rows[i][1]]) if rows[i][2] else XCOL[rows[i][1]]
            j = i + 1
            while j < cnt and j // cols == row and rows[j][1] == rows[i][1] and bool(rows[j][2]) == bool(rows[i][2]):
                j += 1
            o.append(f'<rect x="{ix + col * sw:.2f}" y="{iy + row * sh:.2f}" width="{(j - i) * sw:.2f}" height="{sh:.2f}" fill="{fill}"/>')
            i = j
        if sw >= 3 and sh >= 3:
            for k in range(1, nrows):
                o.append(f'<line x1="{ix:.2f}" y1="{iy + k * sh:.2f}" x2="{ix + iw:.2f}" y2="{iy + k * sh:.2f}" stroke="{BG}" stroke-width="0.5"/>')
        o.append(f'<rect x="{cx:.2f}" y="{cy:.2f}" width="{cw:.2f}" height="{ch:.2f}" fill="none" stroke="{BG}" stroke-width="2"/>')
        label = key.rsplit(".", 1)[0] if key.lower().endswith(".dll") else key
        label = label[: max(0, int((cw - 6) / 7))]
        if label and ch >= 18:
            o.append(text(cx + 5, cy + 16, label))
        o.append("</g>")
    return y + height + 10, cov, n


def badge(name, label, value, colour):
    lw, vw = 7 * len(label) + 12, 7 * len(value) + 12
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{lw + vw}" height="20" role="img" aria-label="{escape(label)}: {escape(value)}">'
           f'<rect width="{lw}" height="20" fill="#555"/><rect x="{lw}" width="{vw}" height="20" fill="{colour}"/>'
           f'<g fill="#fff" font-family="Verdana,DejaVu Sans,sans-serif" font-size="11" text-anchor="middle">'
           f'<text x="{lw / 2}" y="14">{escape(label)}</text><text x="{lw + vw / 2}" y="14">{escape(value)}</text></g></svg>')
    open(os.path.join(img, f"badge-{name}.svg"), "w", encoding="utf-8").write(svg)


def pct_colour(p):
    return "#4c1" if p >= 90 else "#97ca00" if p >= 70 else "#dfb317" if p >= 40 else "#fe7d37"


def metric(d, t):
    return {"done": d, "total": t, "pct": round(100 * d / t, 2) if t else None}




def tcfg(title):
    return next((t for t in cfg["titles_analysed"] if t["name"] == title), {})


def unverified_lists(title):
    tc = tcfg(title)
    ev = evidence("chatprobe.json") or {}
    probe = ev.get("title", "Fortnite") == title
    ub = {b.lower() for b in tc.get("unverified_binaries", []) if not (probe and module_proven("chatprobe.json", b))}
    um = {m.lower() for m in tc.get("unverified_modules", []) if not (probe and module_proven("chatprobe.json", m))}
    return ub, um


def ordinal_evidence(mod, f):
    return cfg.get("ordinal_evidence", {}).get(mod, {}).get(f)


def classify_title_import(title, shipped, binary, mod, f, pcflag, name=None, host=None):
    tc = tcfg(title)
    low = host or mod.lower()
    fn = name or f
    via = f"{mod} -> console host {host}; " if host else (f"{mod} ordinal {f} = {name} on this title's console {mod}; " if name else "")
    emu = {k.lower(): v for k, v in cfg["emulator"].items()}
    hooked = {k: set(v) for k, v in cfg["hooked"].items() if k != "file"}
    ub, um = unverified_lists(title)
    if binary.lower() in ub:
        return "unverified", f"{binary} is delay-loaded by the EXE and never appears in run logs"
    if low in shipped and low in emu and low not in {k.lower() for k in tc.get("emulator_replaces", {})}:
        ef = emu[low]
        return "title", f"shipped with the title ({mod}); the emulator's [{ef['file']}]({TOMBOX}{ef['file']}) also exports it ({ef['functions'].get(fn, 'missing')}) but is not staged for {title}"
    if low in emu and (fn in emu[low]["functions"] or low not in PC_SUPPLEMENT):
        tab = emu_table(emu[low], cached("builds").get(title))
        st = tab["functions"].get(fn, "missing")
        note = (tab.get("notes", {}).get(fn) or cfg["notes"].get(fn, "")) if st != "missing" else f"not exported by the emulator's {host or mod}"
        if low in shipped:
            note = {k.lower(): v for k, v in tc.get("emulator_replaces", {}).items()}.get(low, "") + "; " + note
        src = tab.get("table") or emu[low]["file"]
        return st, via + f"[{src}]({TOMBOX}{src})" + (f" ({note})" if note else "")
    if host:
        cl = next((v for k, v in cached("console").items() if k.lower() == host), {})
        oshook, oshook_file = oshost_hooks()
        if fn in cl.get("twin_has", ()):
            tw = " / ".join(cl.get("pc_twin", ()))
            if fn in oshook.get(host, ()) or any(fn in oshook.get(t, ()) for t in cl.get("pc_twin", ())):
                return "hooked", via + f"PC twin {tw} exports {fn}; hooked by the emulator OS host ({oshook_file})"
            return "pc", via + f"name match on the PC twin {tw}; the console {host} is not emulated"
        return "missing", via + f"no emulator crate for {host} and no PC twin export named {fn}"
    if low in um:
        return "unverified", "delay-loaded; no load seen in run logs"
    if low in shipped:
        return "title", f"shipped with the title ({mod})"
    if f in hooked.get(low, ()):
        return "hooked", f"PC System32\\{mod}; [{cfg['hooked']['file']}]({TOMBOX}{cfg['hooked']['file']})"
    if pcflag == 2:
        o = jload("pc_ordinals.json").get(mod, {}).get(f)
        ce = ordinal_evidence(mod, f)
        if o:
            return "pc", f"PC System32\\{mod} ordinal {f} = {o} on PC" + (f"; {ce}" if ce else " (same meaning on console not verified)")
        return "unverified", f"ordinal import {f} exists on PC {mod}; meaning not checked"
    if pcflag == 1:
        return "pc", f"PC System32\\{mod}" + ("; resolved by the OS loader in our runs" if tc.get("run_evidence") else "; export name match")
    return "missing", f"no emulator export, no PC export named {f} in {mod}"


def title_rows(title, t, full=False):
    shipped = {b.rsplit("/", 1)[-1].lower() for b in t["binaries"]}
    out = []
    for binary in sorted(t["binaries"], key=lambda b: (not b.lower().endswith(".exe"), "/" in b, b.lower())):
        for m in t["binaries"][binary]:
            console_mod = m["module"].lower() not in shipped
            n = len(m["functions"])
            rows = [(f, *classify_title_import(title, shipped, binary, m["module"], f, pf, nm, h))
                    for f, pf, nm, h in zip(m["functions"], m["pc"], m.get("names") or [None] * n, m.get("hosts") or [None] * n)
                    if full or (console_mod and pf == 0)]
            if rows:
                out.append((binary, m, rows))
    return out


def title_excluded(t):
    shipped = {b.rsplit("/", 1)[-1].lower() for b in t["binaries"]}
    pc = own = 0
    for mods in t["binaries"].values():
        for m in mods:
            if m["module"].lower() in shipped:
                own += len(m["functions"])
            else:
                pc += sum(1 for pf in m["pc"] if pf)
    return pc, own


def library_map(summary, libs):
    ti = jload("title_imports.json")
    L = []
    w = L.append
    w("# Library map")
    w("")
    w(f"Generated by `tools/progress/gen.py` from `tools/progress/data/` (refresh with `tools/progress/sync.py`). Upstream: TomBox `{cfg['upstream']['commit']}`; emulator source paths link to upstream. Titles analysed: "
      + ", ".join(f"{k} (console OS {v['os_build']})" if v["os_build"] else f"{k} (no console OS dump; build {v.get('build')} assumed from {v.get('os_build_hint')})" for k, v in ti.items()) + ".")
    w("")
    w("## Scope: Xbox console libraries only")
    w("")
    w("The map counts only what the console provides and a PC does not. A console export is console-related when this PC's `C:\\Windows\\System32` has no DLL of the same name, or has one that does not export that name (for an export by ordinal only: that ordinal). A console DLL that is a modified PC DLL (kernel32, advapi32, mfplat, wldp, ...) therefore counts only for the exports the PC DLL lacks; console DLLs whose every export exists in the PC DLL of the same name (ntdll, msvcrt, ucrtbase, combase, xgameruntime, GameInput, ...) are left out. Title imports are counted the same way: imports that a PC System32 DLL or PC API set provides, and imports from DLLs shipped with the title, are excluded from both done and total. Absence from System32 is the only test, so a few non-Xbox components that happen not to ship with desktop Windows (symsrv, ConIoSrv, KernelTraceControl, the Media Foundation H.264/HEVC decoder DLLs) are counted. Four console host DLLs have no same-named PC DLL but a differently named PC twin (minuserx -> user32, kernel32legacy -> kernel32 / kernelbase, advapi32legacy -> advapi32, minappmodelx -> twinapi.appcore / kernel.appcore); they stay counted in full, and while one is not emulated only its title-imported exports that the twin exports by name are marked `pc` (or `hooked`), so the denominator does not change. kernel32legacy is not treated as a twin: it is emulated by its own compat DLL (crates/compat/kernel32legacy, console ordinals, kernel32-legacy API-set imports routed to it), and every row takes its status from that crate's exports.txt (exports_19041.txt for a title on console OS 19041). Title imports whose console API-set host is one of these four DLLs are counted against that host, not as PC imports.")
    w("")
    w("## Statuses")
    w("")
    w("| Status | Meaning |")
    w("|---|---|")
    w("| implemented | the emulator provides its own implementation |")
    w("| forwarded | an emulator DLL exports it and forwards to a PC system component (D3D12, Media Foundation, DirectStorage) |")
    w("| hooked | PC-twin row (below) that the emulator's OS host also hooks |")
    w("| pc | console host DLL with no same-named PC DLL and no emulator crate (minuserx, advapi32legacy, minappmodelx when not emulated): a title imports the export and its PC twin exports the same name; not implemented by the emulator |")
    w("| stub | the emulator exports it but returns a fixed result (E_NOTIMPL / S_OK / inert object) |")
    w("| unsupported | shown red as \"my PC doesn't support it\": implemented as far as possible, but it needs something this PC does not have (an elevated account, real network access, or a console-only driver), so it cannot be run or verified here |")
    w("| unverified | imported by a binary or module that was never observed loading |")
    w("| missing | no emulator export, and no PC System32 export of that name |")
    w("")
    w("## Headline numbers")
    w("")
    w("Upstream's metric set (same definitions as upstream `tools/progress/gen.py`, Fortnite EXE for system functions), then our additions:")
    w("")
    w("| Metric | Done | Total | % |")
    w("|---|---|---|---|")
    for k, v in list(summary["headline"].items()) + list(summary["extra"].items()):
        if isinstance(v, dict) and "total" in v:
            w(f"| {k} | {v['done'] if v['done'] is not None else 'not published'} | {v['total'] if v['total'] is not None else '-'} | {v['pct'] if v['pct'] is not None else '-'} |")
    w("")
    w("Shader compile: all 139 shaders of the lobby frame compile with DXC (TomBox `docs/DEVELOPMENT_LOG.md:894`); 25/25 fixtures compile (`:672`). The full-library compile sweep (`:706`) has no published final count, so no compile badge is shown.")
    w("")
    w("Direct3D 12.x: 160 / 128 are the table sizes the wrappers allocate (`d3d12_xs/src/device_wrap.rs`, `cmdlist_wrap.rs`), not documented interface sizes. Device: done = forwarded PC slots 0-43 plus intercepted Xbox slots 44, 48, 52 (SetDebugFlags_X, documented no-op), 55, 56/57 and 66/67 (graphics/compute PSO serialize/deserialize, emulator TBPS format only), 60-62, 73, 78-81, 87 (OpenExistingHeapFromAddress via ID3D12Device3); partial = logging S_OK stubs in 45-90, including 74 (RegisterPagePoolX, bookkeeping only). CommandList: done = 0-26, 32-52 (PC methods shifted by 4), 59, 67, 68, 83; 27-31 are Xbox-inserted stubs and 53-127 log-and-ignore probes, counted as not done.")
    w("")
    w("Direct3D 12.x on Xbox One (Terraria, Trackmania; `crates/compat/d3d12_x`): the console `d3d12_x.dll` is a 48 KB loader (export table identical in both dumps, 54 exports, ordinals 1-54) that loads a user-mode driver flavour (`xgs12.dll`, `xgs12_d.dll`, `xgs12_i.dll`) and calls its `UMDCreateDevice12`; Scarlett's `d3d12_xs.dll` loads `xgs12_10*.dll` instead. The COM vtables live in those drivers, not in `d3d12_x.dll`. Neither driver carries RTTI type names for the D3D12 classes. Tables found statically (a constructor `lea` of an `.rdata` table of code pointers, inside a `.pdata` function) number 109 in `xgs12.dll` and 67 in `xgs12_10.dll`; the only one tied to an interface IID through its QueryInterface is ID3D12CommandQueue: 34 slots on Xbox One, 35 on Scarlett. The ID3D12Device and ID3D12GraphicsCommandList tables were not identified by static analysis: both drivers hold long unattributed runs of static code pointers (about 480 entries in `xgs12.dll`, 520 in `xgs12_10.dll`) that may contain them, or the tables may be assembled at run time. So the Xbox One layout of the Xbox-specific slots (Device 44+, CommandList 27-31 and 53+) is unverified. From the titles' own call sites (device pointer traced from the `D3D12XboxCreateDevice` output: Trackmania `[renderer+0x3e80]`, Terraria a global; then `mov rax,[dev]; call [rax+8*n]`), the device slots the titles call are: Trackmania 2, 13, 15, 18-20, 52, 53, 59, 80; Terraria 0, 2, 9, 10, 13-16, 18, 19, 22, 25, 27-30, 36-38, 41, 48, 62, 64, 65, 68, 69, 73, 79-81. Xbox-range slots used by an Xbox One title are therefore 48, 52, 53, 59, 62, 64, 65, 68, 69, 73, 79, 80, 81; their meaning on Xbox One is not yet matched to the Scarlett slots of the same index (Trackmania calls 53 with no arguments and feeds its result to 52 right after creation). Both titles fill the first dword of the 0x28-byte create-parameter block with 0x1000a. The interface IIDs are unchanged (`IID_ID3D12Device` 189819f1-..., the PC GUID), the `D3D12XboxCreateDevice` parameter block is 0x28 bytes and NULL is rejected with E_INVALIDARG. The emulator therefore forwards Xbox One device creation to `d3d12_xs.dll` (Scarlett wrapper): PC slots 0-43 behave as on PC; any Xbox-only slot a title calls goes to the Scarlett slot of the same index, which may be a different method. `D3DMapEsramMemory(Flags, VA, NumPages, PageList)` (signature from the console stub, 0x18-byte driver packet) accepts flags 1 and 3 only and is mapped onto ordinary committed memory of NumPages x 64 KiB at the caller's VA. Xbox One command-list slots seen in Trackmania's recorded lists (`TOMBOX_CMDLIST_SPY`, 11-21) take arguments consistent with the Scarlett mapping: 9 Close, 10 Reset (allocator, null), 21 RSSetViewports (1, ptr), 22 RSSetScissorRects (1, ptr), 26 ResourceBarrier (n, ptr; Xbox state bit 0x400000 OR'd into SR states, stripped for PC), 29 EndQuery (heap, TIMESTAMP, index), 30 ResolveQueryData (heap, TIMESTAMP, start, ...), 32 SetDescriptorHeaps (2, ptr), 50 OMSetRenderTargets (1, ptr, ...), 52 ClearRenderTargetView (handle, color, 0 rects), 83 texture copy (ptr). Xbox One CreateCommittedResource: upload-heap states (0x8c3) become GENERIC_READ, readback states COPY_DEST, and DENY_SHADER_RESOURCE without ALLOW_DEPTH_STENCIL is dropped; ID3D12Resource::GetGPUVirtualAddress on a committed texture returns its CPU-visible backing (unified memory, Unity uses it as the texel pointer), mirrored into the PC texture through the `xg_x` layout.")
    w("")
    counts = {}
    for title, t in ti.items():
        tc = tcfg(title)
        counts[title] = {}
        w(f"## {title} imports" + (f" ({tc['build']})" if tc.get("build") else ""))
        w("")
        if tc.get("summary"):
            w(tc["summary"])
            w("")
        if tc.get("run_evidence"):
            w("Every static import of the EXE and its statically loaded DLLs resolved at load: the process booted and presented frames in the 2026-11-21 runs. Delay imports resolve only when first called.")
        else:
            w("Static analysis only: this title has not been run.")
        w("")
        npc, nown = title_excluded(t)
        w(f"Listed: imports from Xbox console libraries only. Not listed: {npc} imports a PC System32 DLL or API set provides, {nown} imports from DLLs shipped with the title.")
        w("")
        cur = None
        for binary, m, rows in title_rows(title, t):
            if binary != cur:
                w(f"### {title}: {binary}")
                w("")
                cur = binary
            c = {}
            for _, s, _ in rows:
                c[s] = c.get(s, 0) + 1
                counts[title][s] = counts[title].get(s, 0) + 1
            w(f"<details><summary><b>{escape(m['module'])}</b>{' (delay-loaded)' if m['kind'] == 'delay' else ''}: {len(rows)} functions; "
              + ", ".join(f"{s} {c[s]}" for s in XORDER if s in c) + "</summary>")
            w("")
            w("| Function | Status | Implemented in / evidence |")
            w("|---|---|---|")
            for f, s, e in rows:
                w(f"| `{f}` | {s} | {e} |")
            w("")
            w("</details>")
            w("")
        w(f"{title} console-library import totals ({len(t['binaries'])} binaries): " + ", ".join(f"{s} {counts[title][s]}" for s in XORDER if s in counts[title]) + ".")
        w("")
        users = {}
        for o, ot in ti.items():
            if o != title:
                for mods in ot["binaries"].values():
                    for m in mods:
                        for f in m["functions"]:
                            users.setdefault((m["module"].lower(), f), set()).add(o)
        gaps = [(b, m["module"], f, s) for b, m, rows in title_rows(title, t) for f, s, _ in rows if s not in COVERED]
        if gaps:
            w(f"<details><summary><b>{title}: imports not covered</b> ({len(gaps)}; status not implemented / forwarded / hooked / pc / title)</summary>")
            w("")
            w("| Binary | Module | Function | Status | Also imported by |")
            w("|---|---|---|---|---|")
            for b, mod, f, s in sorted(gaps, key=lambda g: (XORDER.index(g[3]), g[1].lower(), g[2].lower())):
                w(f"| {b} | {mod} | `{f}` | {s} | {', '.join(sorted(users.get((mod.lower(), f), ()))) or 'only ' + title} |")
            w("")
            w("</details>")
            w("")
        if len(ti) > 1 and t["os_build"] is None:
            w(f"{title}: no EmbeddedXvd extracted, so no console OS export table of its own; per-build emulator tables and API-set hosts use build {t.get('build')} (from {t.get('os_build_hint')}, not a console OS version).")
            w("")
        elif len(ti) > 1:
            mine = {lib.lower() for lib, v in libs.items() if title in v["versions"]}
            parts = []
            for o in ti:
                if o == title or ti[o]["os_build"] is None:
                    continue
                theirs = {lib.lower() for lib, v in libs.items() if o in v["versions"]}
                extra = sum(1 for lib, v in libs.items() if title in v["versions"] and o in v["versions"]
                            for r in v["rows"] if title in r[3] and o not in r[3])
                same = ti[o]["os_build"] == t["os_build"]
                parts.append(f"vs {o} (OS {ti[o]['os_build']}{', same build' if same else ''}): {len(mine - theirs)} DLLs only here, "
                             f"{len(theirs - mine)} DLLs only there, {extra} exports only here in shared DLLs")
            w(f"{title} console OS {t['os_build']}, {len(mine)} console libraries with console-only exports; " + "; ".join(parts) + ".")
            w("")
    tnames = list(ti)
    w("## Xbox console libraries (EmbeddedXvd, union of all analysed titles)")
    w("")
    w("Console-related exports of the DLLs in each title's EmbeddedXvd `Windows\\System32` and `System32\\forwarders` (union, so the newest console OS defines the surface; see Scope: exports a same-named PC System32 DLL also has are not counted), plus the GDK wrappers the emulator redirects inside the Fortnite EXE (statically linked, not exports; build-specific). The emulator's `mfplat.dll` proxy shadows the PC one for the title, so console mfplat exports it does not forward are counted missing. xgameruntime.dll exports only its dispatcher entry points; the GDK services behind it (XGameSave, XGameUi, XNetworking, ...) are reached via `QueryApiImpl` and are not enumerated here; its exports all exist in the PC xgameruntime.dll, so it is not counted. Import attribution for API-set imports uses the console host from `console_apiset_hosts.txt` when that table lists the function for the title's OS build, otherwise one host DLL (KernelBase, ucrtbase, ntdll, combase, ... first). Export names found as GetProcAddress strings in a title binary that also names the DLL count as title use. Emulator tables with per-OS-build variants (kernel32legacy `exports_19041.txt`) are applied per title build; a union row takes the lowest status across the builds that have the export.")
    w("")
    w("| Library | Folder | Same-named PC DLL | Console-only exports | " + " | ".join(XORDER) + " | " + " | ".join(f"{t} imports" for t in tnames) + " | Versions |")
    w("|---|---|---|---|" + "---|" * len(XORDER) + "---|" * len(tnames) + "---|")
    for lib, v in sorted(libs.items(), key=lambda kv: kv[0].lower()):
        c = {s: 0 for s in XORDER}
        for r in v["rows"]:
            c[r[1]] += 1
        vers = "; ".join(f"{k} {x}" for k, x in v["versions"].items())
        w(f"| {lib} | {v['dir']} | {'yes' if v['pc_dll'] else 'no'} | {len(v['rows'])} | " + " | ".join(str(c[s]) for s in XORDER)
          + " | " + " | ".join(str(sum(1 for r in v["rows"] if t in r[2])) for t in tnames) + f" | {vers} |")
    w("")
    nt_libs, tg_libs = not_targeted(libs)
    nt_n = sum(len(libs[l]["rows"]) for l in nt_libs)
    tg_n = sum(len(libs[l]["rows"]) for l in tg_libs)
    w("### Not targeted (no analysed title references them)")
    w("")
    w(f"The console-library total above ({nt_n + tg_n} exports in {len(libs)} libraries) keeps every console-only export in the denominator, including devkit, tools and OS service DLLs that no analysed title imports. "
      f"{len(tg_libs)} libraries ({tg_n} exports) are imported by at least one title, directly or through API-set host attribution, or are emulator-provided; "
      f"the {len(nt_libs)} below ({nt_n} exports) are not targeted: no title EXE or shipped DLL imports any of their exports and the emulator does not provide them. "
      "They stay `missing` in the headline instead of being dropped from it, so the percentage is not inflated; a library moves out of this list as soon as a title imports it or an emulator crate exports it. "
      f"The earlier Xbox-only figure ({summary['all_including_pc']['xbox_only_exports']['total']} exports) counted only DLLs with no same-named PC DLL and is kept in `summary.json` under `all_including_pc`. "
      f"Coverage inside the {len(tg_libs)} targeted libraries alone: {summary['extra']['console_exports_targeted']['done']}/{tg_n}.")
    w("")
    w("| Library | Folder | Same-named PC DLL | Console-only exports | Present in |")
    w("|---|---|---|---|---|")
    for lib in sorted(nt_libs, key=lambda l: (-len(libs[l]["rows"]), l.lower())):
        v = libs[lib]
        w(f"| {lib} | {v['dir']} | {'yes' if v['pc_dll'] else 'no'} | {len(v['rows'])} | {', '.join(v['versions'])} |")
    w("")
    w("### Evidence files")
    w("")
    w("`sync.py` copies these from TomBox `analysis/progress_evidence/` into `tools/progress/data/evidence/`; `gen.py` reads them, and a missing file changes nothing. Schema: `{\"title\": \"Fortnite\", \"source\": \"<log or test>\", \"modules\": {\"<Module.dll>\": {\"loaded\": true, \"resolved\": [\"<export>\", ...], \"unresolved\": []}}}`. "
      "A module counts as loading and resolving only when `loaded` is true and `unresolved` is an empty list; a system-functions import flips to done only when its name is in `resolved`. `oshost.json` is `{\"file\": \"<crate path>\", \"hooks\": {\"<console or PC twin DLL>\": [\"<export>\", ...]}}`.")
    w("")
    w("| File | Used for | Present |")
    w("|---|---|---|")
    for f, use in (("chatprobe.json", "Fortnite GameChat2.dll / CONCRT140.dll leave the unverified lists; GameChat2 delay import in system functions"),
                   ("media.json", "MFPlat.DLL x16 and MFReadWrite.dll x2 in system functions"),
                   ("gameinput_ext.json", "GameInput.dll imports outside the EXE (TerrariaNative.dll)"),
                   ("oshost.json", "PC-twin rows become `hooked` instead of `pc`")):
        w(f"| `{f}` | {use} | {'yes' if evidence(f) is not None else 'no'} |")
    w("")
    if len(tnames) > 1:
        w("### Exports not present in every title's console OS")
        w("")
        w("Newer console OS builds add exports; each is its own item in the map and stays `missing` until implemented.")
        w("")
        for lib, v in sorted(libs.items(), key=lambda kv: kv[0].lower()):
            part = [r for r in v["rows"] if v["versions"] and set(r[3]) != set(v["versions"])]
            if not part:
                continue
            only = {}
            for r in part:
                only.setdefault(", ".join(r[3]), []).append(r)
            w(f"<details><summary><b>{escape(lib)}</b>: " + "; ".join(f"{len(rs)} only in {k}" for k, rs in only.items()) + "</summary>")
            w("")
            w("| Function | Present in | Status |")
            w("|---|---|---|")
            for k, rs in only.items():
                for r in sorted(rs, key=lambda r: r[0].lower()):
                    w(f"| `{r[0]}` | {k} | {r[1]} |")
            w("")
            w("</details>")
            w("")
    w("### Xbox console libraries, per function")
    w("")
    for lib, v in sorted(libs.items(), key=lambda kv: kv[0].lower()):
        w(f"<details><summary><b>{escape(lib)}</b>: {len(v['rows'])} console-only exports, {sum(1 for r in v['rows'] if r[1] in COVERED)} covered</summary>")
        w("")
        nt = v.get("notes", {})
        if v.get("pc_twin") and v.get("emulated"):
            w(f"No PC DLL is named {lib}; it is emulated by its compat crate, so no row is a PC-twin name match: every status comes from the crate's exports.txt.")
            w("")
        elif v.get("pc_twin"):
            w(f"No PC DLL is named {lib}; its PC twin is {' / '.join(v['pc_twin'])}. Exports an analysed title imports and the twin exports by name are `pc` (or `hooked` where the emulator's OS host reports a hook): {len(nt)} rows. They are not implemented by the emulator; the other exports stay `missing`.")
            w("")
        w("| Function | Status | Imported by | Present in |" + (" Note |" if nt else ""))
        w("|---|---|---|---|" + ("---|" if nt else ""))
        for r in sorted(v["rows"], key=lambda r: (XORDER.index(r[1]), r[0].lower())):
            w(f"| `{r[0]}` | {r[1]} | {', '.join(sorted(r[2]))} | {', '.join(r[3])} |" + (f" {nt.get(r[0], '')} |" if nt else ""))
        w("")
        w("</details>")
        w("")
    w("## Verified on this machine (running titles)")
    w("")
    w("| Milestone | Evidence |")
    w("|---|---|")
    for m in cfg["verified_milestones"]:
        w(f"| {m['name']} | `{m['evidence']}` |")
    for m in cfg["open_milestones"]:
        w(f"| {om_name(m)} | {om_state(m)} |")
    w("")
    w("## Unverified")
    w("")
    ub, um = unverified_lists("Fortnite")
    if "gamechat2.dll" in ub or "gamechat2.dll" in um:
        w("- Fortnite: GameChat2.dll (14 delay imports) and everything it imports: never seen loading.")
    else:
        w("- Fortnite: GameChat2.dll loads and resolves (chatprobe evidence, `tools/progress/data/evidence/chatprobe.json`).")
    if "concrt140.dll" in um:
        w("- Fortnite: CONCRT140.dll (MSVCP140 delay import): not observed.")
    else:
        w("- Fortnite: CONCRT140.dll loads and resolves (chatprobe evidence).")
    for mod, fs in cfg.get("ordinal_evidence", {}).items():
        w(f"- {mod} ordinal imports: " + "; ".join(f"{f}: {e}" for f, e in fs.items()) + ".")
    w("- Ordinal imports (XAudio2_9 #1/#2/#4/#5/#6, GameInput #1) are resolved to names with the console DLL of the importing title's own EmbeddedXvd, then classified like named imports: XAudio2_9 against crates/compat/xaudio2_9 (its table is checked against the built target/release/XAudio2_9.dll), GameInput against the PC DLL. A title without an EmbeddedXvd (Minecraft) keeps the `#N` form. XAudio2 starts streaming in the Fortnite log, but there is no audio.")
    w("- GetProcAddress-string usage: an export name counts as used by a title when a binary of that title holds it as an exact ASCII string (not preceded by a printable byte, NUL-terminated) and also names the DLL (ASCII or UTF-16). It marks the console row as imported (lighter colour, targeted), but it is not a load observed at run time and does not enter the per-title import tables.")
    w("- API-set imports whose console host (`crates/compat/compat-common/data/console_apiset_hosts.txt`, per OS build with the runtime's nearest-lower-build rule) is minuserx, advapi32legacy, kernel32legacy or minappmodelx are listed against that host; API-set functions the host table does not list keep the PC name-match treatment, and that table is partial.")
    w("- PC-twin rows (minuserx -> user32, advapi32legacy -> advapi32, minappmodelx -> twinapi.appcore / kernel.appcore, for whichever of them has no emulator crate) are export-name matches on the twin only; the console DLLs' own behaviour is not reproduced. kernel32legacy is emulated by crates/compat/kernel32legacy and has no PC-twin rows.")
    w("- PC-provided is an export-name match (ordinal match for ordinal-only exports) against this PC's System32; behaviour differences between console and PC system DLLs are not measured, and such exports are outside this map.")
    w("- Titles without run evidence are classified statically only.")
    w("- Milestones are counted live by gen.py (mapping milestones from this map's sections, run milestones from `progress.json`); see `docs/MILESTONES.md`.")
    w("")
    open(os.path.join(root, "docs", "LIBRARY_MAP.md"), "w", encoding="utf-8").write("\n".join(L))
    return counts


def selftest():
    global EVIDENCE_OVERRIDE
    fails = []

    def check(name, ok):
        print(("ok   " if ok else "FAIL ") + name)
        if not ok:
            fails.append(name)
    saved = EVIDENCE_OVERRIDE
    EVIDENCE_OVERRIDE = {}
    base = console(twins=False)
    tw = console()
    rows0 = [r for v in base.values() for r in v["rows"]]
    rows1 = [r for v in tw.values() for r in v["rows"]]
    check("twin mapping keeps the console denominator", len(rows0) == len(rows1))
    check("twin mapping keeps the library set and pc_dll flags", {k: v["pc_dll"] for k, v in base.items()} == {k: v["pc_dll"] for k, v in tw.items()})
    check("twin libraries present", {k.lower() for k, v in tw.items() if v.get("pc_twin")} >= {"minuserx.dll", "kernel32legacy.dll", "advapi32legacy.dll", "minappmodelx.dll"})
    twl = [k for k, v in tw.items() if v.get("pc_twin") and k.lower() not in {e.lower() for e in cfg["emulator"]}]
    flips = [(k, a[0], b[1]) for k in twl for a, b in zip(base[k]["rows"], tw[k]["rows"]) if a[1] != b[1]]
    check("only imported twin rows flip", all(next(r for r in tw[k]["rows"] if r[0] == n)[2] for k, n, _ in flips))
    check("twin rows flip to pc only (no oshost evidence)", all(s == "pc" for _, _, s in flips))
    check("no twin row is implemented", not any(r[1] == "implemented" for k in twl for r in tw[k]["rows"]))
    check("every twin flip carries a note", all(n in tw[k]["notes"] for k, n, _ in flips))
    _emu_twins = {"kernel32legacy.dll", "advapi32legacy.dll"} & {e.lower() for e in cfg["emulator"]}
    _mux_emu = "minuserx.dll" in {e.lower() for e in cfg["emulator"]}
    _mam_emu = "minappmodelx.dll" in {e.lower() for e in cfg["emulator"]}
    check("35 twin rows flip (25 once kernel32legacy and advapi32legacy are emulated, 21 fewer once minuserx is, 4 fewer once minappmodelx is)", len(flips) == (25 if len(_emu_twins) == 2 else 35) - (21 if _mux_emu else 0) - (4 if _mam_emu else 0))
    cov0 = sum(1 for r in rows0 if r[1] in COVERED)
    cov1 = sum(1 for r in rows1 if r[1] in COVERED)
    check("covered rises by exactly the twin flips", cov1 - cov0 == len(flips))
    if flips:
        hl, hk = next((k, n) for k, n, _ in sorted(flips, key=lambda f: f[0].lower() != "minuserx.dll"))
        EVIDENCE_OVERRIDE = {"oshost.json": {"file": "crates/compat/oshost/src/lib.rs", "hooks": {tw[hl]["pc_twin"][0].lower(): [hk]}}}
        h = console()
        st = {r[0]: r[1] for k, v in h.items() if k == hl for r in v["rows"]}
        check("oshost hook on the PC twin makes the row hooked", st[hk] == "hooked")
    else:
        print("skip oshost hook on the PC twin makes the row hooked (every twin library is emulated, no PC-twin row left)")
    EVIDENCE_OVERRIDE = {}
    g = imports()
    check("GameInput #1 done without touching XAudio2", g.get("GameInput") == ["done"] and g.get("XAudio2_9", []).count(cfg["import_overrides"].get("XAudio2_9.dll", "done")) == 5)
    check("GameChat2 stays none without chatprobe evidence", g.get("GameChat2") == ["none"])
    check("MFPlat stays partial without media evidence", g.get("MFPlat", []).count("partial") == 16)
    check("unverified lists unchanged without evidence", unverified_lists("Fortnite") == ({"gamechat2.dll"}, {"gamechat2.dll", "concrt140.dll"}))
    exe = jload("fortnite_exe.json")
    mf = next(m["functions"] for m in exe["imports"] if m["module"].lower() == "mfplat.dll")
    EVIDENCE_OVERRIDE = {"media.json": {"modules": {"MFPlat.DLL": {"loaded": True, "resolved": mf[:-1], "unresolved": [mf[-1]]}}},
                         "chatprobe.json": {"title": "Fortnite", "modules": {"GameChat2.dll": {"loaded": True, "unresolved": []},
                                                                             "CONCRT140.dll": {"loaded": True, "unresolved": ["x"]}}}}
    g = imports()
    check("media evidence flips only resolved MFPlat imports", g["MFPlat"].count("done") == 15 and g["MFPlat"].count("partial") == 1)
    check("chatprobe evidence flips the GameChat2 delay import", g["GameChat2"] == ["done"])
    check("chatprobe evidence removes GameChat2 but keeps unresolved CONCRT140", unverified_lists("Fortnite") == (set(), {"concrt140.dll"}))
    EVIDENCE_OVERRIDE = {"chatprobe.json": {"title": "Fortnite", "modules": {"GameChat2.dll": {"loaded": False, "unresolved": []}}}}
    check("chatprobe with loaded false changes nothing", unverified_lists("Fortnite")[0] == {"gamechat2.dll"})
    EVIDENCE_OVERRIDE = saved
    ti = jload("title_imports.json")
    fn = ti.get("Fortnite", {})
    full = {(b, m, f): (s_, e) for b, mm, rs in title_rows("Fortnite", fn, full=True) for f, s_, e in rs for m in [mm["module"]]} if fn else {}
    xa = full.get(("FortniteClient-XSX-Shipping.exe", "XAudio2_9.dll", "#1"))
    check("XAudio2_9 #1 links to the console ordinal name XAudio2Create", xa is not None and "XAudio2Create" in xa[1] and xa[0] == cfg["emulator"].get("xaudio2_9.dll", {}).get("functions", {}).get("XAudio2Create", "pc"))
    hosted = [(f, s_, e) for (b, m, f), (s_, e) in full.items() if "console host" in e]
    check("console-hosted API-set imports are listed against their console host", any(f == "PostMessageW" and "minuserx.dll" in e for f, s_, e in hosted)
          and any(f == "RegOpenKeyW" and "advapi32legacy.dll" in e for f, s_, e in hosted))
    k32 = cfg["emulator"].get("kernel32legacy.dll", {})
    if "19041" in k32.get("builds", {}) and fn.get("build") == 19041:
        sta = [e for f, s_, e in hosted if "kernel32legacy.dll" in e]
        check("Fortnite (OS 19041) kernel32legacy rows use the 19041 table", bool(sta) and all("exports_19041.txt" in e for e in sta))
    ahw = next((v for k, v in tw.items() if k.lower() == "audiohalwrapper.dll"), None)
    dyn = {f for t_ in ti.values() for bins in t_.get("dynamic", {}).values() for lib, fs in bins.items() if lib.lower() == "audiohalwrapper.dll" for f in fs}
    check("GetProcAddress-string usage marks AudioHALWrapper rows as title-used", ahw is not None and bool(dyn) and all(r[2] for r in ahw["rows"] if r[0] in dyn))
    cc = os.path.join(dat, "emulator_cfg_checks.json")
    for dll_, c_ in (json.load(open(cc, encoding="utf-8")) if os.path.isfile(cc) else {}).items():
        for f_ in c_.get("not_in_built_dll", []):
            check(f"{dll_}!{f_} is not counted as emulator-provided (absent from {c_['built_dll']})", f_ not in cfg["emulator"].get(dll_.lower(), {}).get("functions", {}))
    nt, tg = not_targeted(tw)
    check("not-targeted + targeted = console denominator", sum(len(tw[l]["rows"]) for l in nt + tg) == len(rows1))
    check("not-targeted libraries have no title import", not any(r[2] for l in nt for r in tw[l]["rows"]))
    print(f"{len(fails)} failed")
    return not fails


if "--selftest" in sys.argv:
    sys.exit(0 if selftest() else 1)
os.makedirs(img, exist_ok=True)
o, y = [], 0
y, a_done, a_n = section(o, y, "Imported system functions*", imports(), 300)
y, b_done, b_n = section(o, y, "Direct3D 12.x interface methods", d3d12x(), 170)
y, c_done, c_n = section(o, y, "RDNA2 shader instructions translated", instructions(), 260)
stages = {k: [v] for k, v in cfg["stages"].items()}
y, d_done, d_n = section(o, y, "Shader stages translated", stages, 60)
libs = console()
y, e_done, e_n = console_section(o, y, libs, 900)
svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 {y:.0f}" width="1000" height="{y:.0f}">\n'
       f'<rect x="0" y="0" width="1000" height="{y:.0f}" fill="{BG}"/>\n' + "\n".join(o) + "\n</svg>\n")
open(os.path.join(img, "progress.svg"), "w", encoding="utf-8").write(svg)
ti_all = jload("title_imports.json")
examined = sorted(set(cfg["titles_upstream"]) | set(ti_all))
s = cfg["shaders"]
map_ms = [
    ("All imported system functions covered", a_done, a_n),
    ("All Direct3D 12.x methods covered", b_done, b_n),
    ("All RDNA2 instructions verified", c_done, c_n),
    ("All shader stages translated", d_done, d_n),
    ("Full title shader sweep translated", s["translated"], s["total"]),
    ("All Xbox console libraries covered", e_done, e_n),
]
run_done = len(cfg["verified_milestones"])
run_total = run_done + len(cfg["open_milestones"])
map_done = sum(1 for _, dn, tn in map_ms if dn == tn)
m = {"done": run_done + map_done, "total": run_total + len(map_ms)}
with open(os.path.join(root, "docs", "MILESTONES.md"), "w", encoding="utf-8") as f:
    f.write("# Milestones\n\n")
    f.write(f"{m['done']}/{m['total']} reached (generated by `tools/progress/gen.py`).\n\n")
    f.write("## Mapping\n\n| Milestone | Progress | State |\n|---|---|---|\n")
    for name, dn, tn in map_ms:
        f.write(f"| {name} | {dn}/{tn} | {'reached' if dn == tn else 'open'} |\n")
    f.write("\n## Running titles\n\n| Milestone | Evidence |\n|---|---|\n")
    for vm_ in cfg["verified_milestones"]:
        f.write(f"| {vm_['name']} | `{vm_['evidence']}` |\n")
    for om in cfg["open_milestones"]:
        f.write(f"| {om_name(om)} | {om_state(om)} |\n")
badge("imports", "system functions*", f"{100 * a_done / a_n:.2f}%", pct_colour(100 * a_done / a_n))
badge("d3d12x", "Direct3D 12.x methods", f"{100 * b_done / b_n:.2f}%", pct_colour(100 * b_done / b_n))
badge("instructions", "RDNA2 instructions", f"{100 * c_done / c_n:.2f}%", pct_colour(100 * c_done / c_n))
badge("shaders", "shaders translated", f'{s["translated"]:,}/{s["total"]:,}', "#4c1")
badge("stages", "shader stages", f"{d_done}/{d_n}", pct_colour(100 * d_done / d_n))
badge("titles", "titles examined", str(len(examined)), "#007ec6")
badge("milestones", "milestones", f'{m["done"]}/{m["total"]}', pct_colour(100 * m["done"] / m["total"]))
badge("console", "Xbox console libraries", f"{100 * e_done / e_n:.2f}%", pct_colour(100 * e_done / e_n))
vm = len(cfg["verified_milestones"])
summary = {
    "updated": cfg["updated"],
    "headline": {
        "system_functions": metric(a_done, a_n),
        "d3d12x_methods": metric(b_done, b_n),
        "rdna2_instructions": metric(c_done, c_n),
        "shaders_translated": metric(s["translated"], s["total"]),
        "shader_stages": metric(d_done, d_n),
        "milestones": metric(m["done"], m["total"]),
        "titles_examined": {"done": len(examined), "total": None, "pct": None, "titles": examined},
    },
    "extra": {
        "console_exports": metric(e_done, e_n),
        "shader_compile_lobby_frame": metric(cfg["shader_compile"]["lobby_frame"]["done"], cfg["shader_compile"]["lobby_frame"]["total"]),
        "shader_compile_fixtures": metric(cfg["shader_compile"]["fixtures"]["done"], cfg["shader_compile"]["fixtures"]["total"]),
        "shader_compile_full_sweep": {"done": None, "total": cfg["shader_compile"]["full_sweep"]["total"], "pct": None},
        "milestones_verified_this_machine": metric(vm, vm + len(cfg["open_milestones"])),
    },
}
for title, t in ti_all.items():
    rows = [r for _, _, rs in title_rows(title, t) for r in rs]
    summary["extra"][f"title_imports_{title}"] = metric(sum(1 for r in rows if r[1] in COVERED), len(rows))
_nt, _tg = not_targeted(libs)
_tgr = [r for l in _tg for r in libs[l]["rows"]]
summary["extra"]["console_exports_targeted"] = metric(sum(1 for r in _tgr if r[1] in COVERED), len(_tgr))
summary["extra"]["console_exports_not_targeted"] = {"done": None, "total": sum(len(libs[l]["rows"]) for l in _nt), "pct": None, "libraries": len(_nt)}
old_libs = console(full=True)
old_rows = [r for v in old_libs.values() for r in v["rows"]]
old_xo = [r for v in old_libs.values() if not v["pc_dll"] for r in v["rows"]]
summary["all_including_pc"] = {
    "note": "previous definition, kept for history: every export of every console OS DLL and every title import, PC-provided ones included",
    "console_exports": metric(sum(1 for r in old_rows if r[1] in COVERED), len(old_rows)),
    "xbox_only_exports": metric(sum(1 for r in old_xo if r[1] in COVERED), len(old_xo)),
    "title_import_status": {},
}
for title, t in ti_all.items():
    rows = [r for _, _, rs in title_rows(title, t, full=True) for r in rs]
    summary["all_including_pc"][f"title_imports_{title}"] = metric(sum(1 for r in rows if r[1] in COVERED), len(rows))
    st = {}
    for r in rows:
        st[r[1]] = st.get(r[1], 0) + 1
    summary["all_including_pc"]["title_import_status"][title] = st
tcounts = library_map(summary, libs)
summary["title_import_status"] = tcounts
summary["console_os_builds"] = {k: v["os_build"] for k, v in ti_all.items()}
json.dump(summary, open(os.path.join(here, "summary.json"), "w", encoding="utf-8"), indent=1)
print(f"imports {a_done}/{a_n}  d3d12x {b_done}/{b_n}  instructions {c_done}/{c_n}  stages {d_done}/{d_n}  "
      f"console {e_done}/{e_n}  titles {examined}  height {y:.0f}  bytes {len(svg)}")
print(json.dumps({k: v for k, v in summary["extra"].items() if k.startswith("title_imports")}))
