import argparse, os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MANAGED = ["crates", os.path.join("tools", "dev"), os.path.join("tools", "qa"), os.path.join("tools", "verify"),
           os.path.join("tools", "snap_window.ps1")]

TOP_FILES = {"Cargo.toml", "Cargo.lock"}
TOP_DIRS = {"crates", "tools"}
SKIP_DIRS = {"target", "batch_runs", "evidence", "node_modules", ".git", "__pycache__", "out", "runs", "backend_data", ".pr"}
SKIP_EXT = {".md", ".log", ".exe", ".dll", ".pdb", ".lib", ".obj", ".o", ".a", ".bin", ".dxil", ".cso", ".zip",
            ".7z", ".rar", ".xvd", ".xvc", ".msixvc", ".png", ".jpg", ".jpeg", ".gif", ".mp4", ".webm", ".wav",
            ".xma", ".etl", ".dmp", ".pak", ".ucas", ".utoc", ".sys", ".so", ".dylib", ".rlib", ".pyc", ".ilk", ".exp", ".diff", ".patch"}
MAX_BYTES = 2 * 1024 * 1024
PATTERN_FILE = os.path.join(".git", "info", "private-patterns")


def git(repo, *args, check=True):
    r = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError("git " + " ".join(args) + " failed: " + r.stderr.strip())
    return r


def load_bad(repo):
    path = os.path.join(repo, PATTERN_FILE)
    if not os.path.isfile(path):
        sys.exit("refusing to publish: " + PATTERN_FILE + " is missing. It holds the private-string patterns "
                 "(one regex per line) that must never be published; create it in this clone first.")
    pats = []
    with open(path, encoding="utf-8") as h:
        for line in h:
            line = line.strip()
            if line and not line.startswith("#"):
                pats.append(line)
    if not pats:
        sys.exit("refusing to publish: " + PATTERN_FILE + " has no patterns")
    return re.compile("(?i)" + "|".join("(?:" + p + ")" for p in pats))


def unsynced_contributions(repo, fetch=True):
    if git(repo, "remote", check=False).stdout.split().count("origin") == 0:
        return []
    if fetch:
        r = git(repo, "fetch", "-q", "origin", check=False)
        if r.returncode != 0:
            print("warning: git fetch failed, checking against the last fetched origin/main:", r.stderr.strip())
    if git(repo, "rev-parse", "-q", "--verify", "refs/remotes/origin/main", check=False).returncode != 0:
        return []
    out = git(repo, "log", "--format=%h %an: %s", "HEAD..origin/main").stdout
    return [l for l in out.splitlines() if l.strip()]


def wanted(rel):
    parts = rel.replace("\\", "/").split("/")
    if len(parts) == 1:
        return parts[0] in TOP_FILES
    if parts[0] not in TOP_DIRS:
        return False
    if parts[0] == "tools" and len(parts) > 2 and parts[1] == "progress":
        return False
    for p in parts[:-1]:
        if p in SKIP_DIRS or p.startswith("target"):
            return False
    name = parts[-1]
    ext = os.path.splitext(name)[1].lower()
    if ext in SKIP_EXT or name.lower().endswith((".txt.bak", ".orig")):
        return False
    return True


def main():
    ap = argparse.ArgumentParser(description="Mirror the emulator source tree into the public repository.")
    ap.add_argument("--repo", default=HERE)
    ap.add_argument("--src", default=None)
    ap.add_argument("--no-fetch", action="store_true")
    a = ap.parse_args()
    dst = os.path.abspath(a.repo)
    src = os.path.abspath(a.src or os.path.join(dst, "emulator"))
    if not os.path.isdir(os.path.join(src, "crates")):
        sys.exit("refusing to publish: " + src + " has no crates directory")
    bad = load_bad(dst)
    pending = unsynced_contributions(dst, fetch=not a.no_fetch)
    if pending:
        print("refusing to publish: origin/main has %d commit(s) that are not in this clone yet." % len(pending))
        print("Publishing now would overwrite contributors' merged work. Run tools/pull_contrib.py first.")
        for l in pending[:20]:
            print("  " + l)
        sys.exit(2)
    for m in MANAGED + sorted(TOP_FILES):
        p = os.path.join(dst, m)
        if os.path.isdir(p):
            shutil.rmtree(p)
        elif os.path.isfile(p):
            os.remove(p)
    copied, blocked = 0, []
    for root, dirs, files in os.walk(src):
        rel_root = os.path.relpath(root, src)
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS and not d.startswith(("target", "run"))]
        for f in files:
            rel = f if rel_root == "." else os.path.join(rel_root, f)
            if not wanted(rel):
                continue
            path = os.path.join(root, f)
            if os.path.getsize(path) > MAX_BYTES:
                continue
            with open(path, "rb") as h:
                data = h.read()
            if b"\0" in data[:8192]:
                continue
            text = data.decode("utf-8", "replace")
            if bad.search(text):
                blocked.append(rel)
                continue
            out = os.path.join(dst, rel)
            os.makedirs(os.path.dirname(out), exist_ok=True)
            with open(out, "wb") as h:
                h.write(data)
            copied += 1
    print("copied", copied)
    for b in blocked:
        print("blocked (personal info):", b)


if __name__ == "__main__":
    main()
