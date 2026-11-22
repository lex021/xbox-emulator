import argparse, datetime, json, os, subprocess, sys, tempfile

sys.dont_write_bytecode = True

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import publish_source as pub

MARKER = os.path.join(".git", "contrib-sync.json")
CONFLICT_DIR = os.path.join(".git", "contrib-conflicts")


def git(repo, *args, check=True, binary=False):
    r = subprocess.run(["git", "-C", repo, *args], capture_output=True)
    if check and r.returncode != 0:
        raise SystemExit("git " + " ".join(args) + " failed: " + r.stderr.decode("utf-8", "replace").strip())
    if binary:
        return r
    r.stdout = r.stdout.decode("utf-8", "replace")
    r.stderr = r.stderr.decode("utf-8", "replace")
    return r


def rev(repo, ref):
    r = git(repo, "rev-parse", "-q", "--verify", ref + "^{commit}", check=False)
    return r.stdout.strip() if r.returncode == 0 else None


def is_ancestor(repo, a, b):
    return git(repo, "merge-base", "--is-ancestor", a, b, check=False).returncode == 0


def blob(repo, commit, path):
    r = git(repo, "show", commit + ":" + path, check=False, binary=True)
    return r.stdout if r.returncode == 0 else None


def managed(path):
    p = path.replace("\\", "/")
    for m in pub.MANAGED:
        m = m.replace("\\", "/")
        if p == m or p.startswith(m + "/"):
            return True
    return p in pub.TOP_FILES


def is_binary(data):
    return data is not None and b"\0" in data[:8192]


def to_eol(data, crlf):
    if data is None:
        return None
    lf = data.replace(b"\r\n", b"\n")
    return lf.replace(b"\n", b"\r\n") if crlf else lf


def same(a, b):
    if a is None or b is None:
        return a is b
    return a.replace(b"\r\n", b"\n") == b.replace(b"\r\n", b"\n")


def merge3(repo, ours, base, theirs, path):
    with tempfile.TemporaryDirectory() as d:
        names = []
        for tag, data in (("ours", ours), ("base", base), ("theirs", theirs)):
            p = os.path.join(d, tag)
            with open(p, "wb") as h:
                h.write(data or b"")
            names.append(p)
        r = subprocess.run(["git", "merge-file", "-p", "-L", "emulator/" + path, "-L", "published", "-L", "contributor",
                            *names], capture_output=True)
    if r.returncode < 0 or r.returncode > 127:
        return None, -1
    return r.stdout, r.returncode


def changed_paths(repo, base, theirs):
    out = git(repo, "diff", "--name-status", "--no-renames", "-z", base, theirs).stdout
    items = out.split("\0")
    res = []
    i = 0
    while i + 1 < len(items):
        st, path = items[i], items[i + 1]
        if st:
            res.append((st[0], path))
        i += 2
    return res


def plan(repo, src, base, theirs):
    actions, conflicts, notes, native = [], [], [], []
    for st, path in changed_paths(repo, base, theirs):
        low = path.replace("\\", "/").lower()
        if low.startswith("games/") or "/games/" in low:
            notes.append(("skipped game path", path))
            continue
        if not managed(path):
            native.append((st, path))
            continue
        b = blob(repo, base, path)
        t = blob(repo, theirs, path)
        if is_binary(t) or os.path.splitext(path)[1].lower() in pub.SKIP_EXT and os.path.splitext(path)[1].lower() != ".md":
            notes.append(("binary not imported (the next publish removes it from the repo)", path))
            continue
        if not pub.wanted(path):
            notes.append(("not publishable (.md, filtered dir or extension): not imported, the next publish removes it", path))
            continue
        if t is not None and len(t) > pub.MAX_BYTES:
            notes.append(("over the publish size limit: not imported, the next publish removes it", path))
            continue
        target = os.path.join(src, path)
        o = None
        if os.path.isfile(target):
            with open(target, "rb") as h:
                o = h.read()
        if st == "D" or t is None:
            if o is None:
                actions.append(("already", path, None))
            elif same(o, b):
                actions.append(("delete", path, None))
            else:
                conflicts.append((path, "deleted by the contributor, changed in emulator/", None))
            continue
        if o is None:
            if st == "A" or b is None:
                actions.append(("add", path, t))
            else:
                conflicts.append((path, "changed by the contributor, deleted in emulator/", None))
            continue
        if same(o, t):
            actions.append(("already", path, None))
            continue
        crlf = b"\r\n" in o
        merged, n = merge3(repo, o, to_eol(b, crlf) if b is not None else b"", to_eol(t, crlf), path)
        if n < 0:
            conflicts.append((path, "git merge-file cannot merge it (binary content in one side)", None))
        elif n:
            conflicts.append((path, "%d conflicting hunk(s)" % n, merged))
        elif same(merged, o):
            actions.append(("already", path, None))
        else:
            actions.append(("merge", path, merged))
    return actions, conflicts, notes, native


def read_marker(repo):
    p = os.path.join(repo, MARKER)
    if os.path.isfile(p):
        with open(p, encoding="utf-8") as h:
            return json.load(h)
    return {}


def write_marker(repo, data):
    with open(os.path.join(repo, MARKER), "w", encoding="utf-8") as h:
        json.dump(data, h, indent=1)


def git_merge(repo, theirs):
    head = rev(repo, "HEAD")
    if is_ancestor(repo, theirs, head):
        return True, "HEAD already contains " + theirs[:10]
    if is_ancestor(repo, head, theirs):
        r = git(repo, "merge", "--ff-only", "-q", theirs, check=False)
    else:
        t = git(repo, "merge-tree", "--write-tree", "--name-only", head, theirs, check=False)
        if t.returncode != 0:
            return False, "merging origin/main into the local branch would conflict:\n" + t.stdout
        r = git(repo, "merge", "--no-ff", "--no-edit", "-q", theirs, check=False)
    if r.returncode != 0:
        return False, (r.stdout + r.stderr).strip()
    return True, "local branch now at " + rev(repo, "HEAD")[:10]


def main():
    ap = argparse.ArgumentParser(description="Merge contributor commits from origin/main back into emulator/.")
    ap.add_argument("--repo", default=pub.HERE)
    ap.add_argument("--src", default=None)
    ap.add_argument("--ref", default="origin/main")
    ap.add_argument("--base", default=None)
    ap.add_argument("--no-fetch", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--no-git-merge", action="store_true")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    repo = os.path.abspath(a.repo)
    src = os.path.abspath(a.src or os.path.join(repo, "emulator"))
    if not os.path.isdir(os.path.join(src, "crates")):
        sys.exit("no crates directory in " + src)
    if not a.no_fetch:
        r = git(repo, "fetch", "-q", "origin", check=False)
        if r.returncode != 0:
            sys.exit("git fetch failed: " + r.stderr.strip())
    theirs = rev(repo, a.ref)
    if not theirs:
        sys.exit("unknown ref " + a.ref)
    head = rev(repo, "HEAD")
    base = rev(repo, a.base) if a.base else rev(repo, git(repo, "merge-base", head, theirs).stdout.strip())
    marker = read_marker(repo)
    result = {"ref": a.ref, "theirs": theirs, "base": base, "head": head, "actions": [], "conflicts": [], "notes": [],
              "repo_native": []}
    if is_ancestor(repo, theirs, head) and not a.base:
        result["status"] = "up-to-date"
        report(result, a.json)
        return 0
    if not a.dry_run and a.ref == "origin/main" and marker.get("synced_to") == theirs:
        result["status"] = "emulator already synced to " + theirs[:10]
    else:
        actions, conflicts, notes, native = plan(repo, src, base, theirs)
        result["actions"] = [(k, p) for k, p, _ in actions]
        result["conflicts"] = [(p, why) for p, why, _ in conflicts]
        result["notes"] = notes
        result["repo_native"] = native
        if conflicts:
            if not a.dry_run:
                cdir = os.path.join(repo, CONFLICT_DIR)
                for p, why, merged in conflicts:
                    if merged is not None:
                        out = os.path.join(cdir, p)
                        os.makedirs(os.path.dirname(out), exist_ok=True)
                        with open(out, "wb") as h:
                            h.write(merged)
                result["conflict_dir"] = cdir
            result["status"] = "conflicts: nothing written to emulator/"
            report(result, a.json)
            return 1
        if a.dry_run:
            result["status"] = "dry run: clean"
            report(result, a.json)
            return 0
        for kind, path, data in actions:
            target = os.path.join(src, path)
            if kind in ("add", "merge"):
                os.makedirs(os.path.dirname(target), exist_ok=True)
                with open(target, "wb") as h:
                    h.write(data)
            elif kind == "delete":
                os.remove(target)
        write_marker(repo, {"synced_to": theirs, "base": base, "head_before": head,
                            "time": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
                            "files": result["actions"]})
        result["status"] = "emulator/ synced to " + theirs[:10]
    if a.dry_run or a.no_git_merge or a.ref != "origin/main":
        report(result, a.json)
        return 0
    ok, msg = git_merge(repo, theirs)
    result["git_merge"] = msg
    if ok:
        m = read_marker(repo)
        m["published_base"] = theirs
        write_marker(repo, m)
    report(result, a.json)
    return 0 if ok else 3


def report(r, as_json):
    if as_json:
        print(json.dumps(r, indent=1))
        return
    print("contributor ref", r["ref"], r["theirs"][:10], "base", (r["base"] or "")[:10], "head", r["head"][:10])
    for k, p in r["actions"]:
        print("  %-8s %s" % (k, p))
    for p, why in r["conflicts"]:
        print("  CONFLICT %s: %s" % (p, why))
    for why, p in r["notes"]:
        print("  note     %s: %s" % (p, why))
    for st, p in r["repo_native"]:
        print("  repo     %s %s (repository file, arrives with the git merge)" % (st, p))
    if r.get("conflict_dir"):
        print("merged files with conflict markers are in", r["conflict_dir"])
    if r.get("git_merge"):
        print("git merge:", r["git_merge"])
    print(r["status"])


if __name__ == "__main__":
    sys.exit(main())
