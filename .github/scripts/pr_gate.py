import argparse, os, re, subprocess, sys

BINARY_EXT = {".exe", ".dll", ".pdb", ".bin", ".lib", ".obj", ".o", ".a", ".so", ".dylib", ".rlib", ".sys", ".ilk",
              ".exp", ".pyc", ".dxil", ".cso", ".dxbc", ".spv", ".fxc", ".xvd", ".xvc", ".msixvc", ".pak", ".ucas",
              ".utoc", ".rpf", ".ttpf", ".fssm", ".xex", ".xbe", ".iso", ".dmp", ".etl", ".zip", ".7z", ".rar", ".cab",
              ".appx", ".msix", ".msixbundle", ".dds", ".xma", ".wem", ".bnk", ".awc", ".bk2", ".ogg", ".wav", ".mp3",
              ".mp4", ".webm", ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tga", ".ytd", ".ydr", ".yft", ".ymap",
              ".key", ".pfx", ".p12", ".pem"}
GAME_PATH = re.compile(r"(?i)(^|/)(games|dumps|dump|input|title_dumps|gamedata)/|(^|/)(microsoftgame\.config|appxmanifest\.xml|"
                       r"layout\.xml|xbox_?one_?license.*|license\.xml)$")
MAX_BYTES = 1024 * 1024
LARGE_OK = re.compile(r"^(tools/progress/|docs/LIBRARY_MAP\.md$|Cargo\.lock$)")
PRIVATE = [
    (re.compile(r"(?i)\b[a-z]:[\\/]{1,2}users[\\/]{1,2}(?!public\b|default\b|all users\b|someone\b|<)[^\\/\s\"'`]+"), "absolute path into a Windows user profile"),
    (re.compile(r"(?<![\w.])/home/(?!runner\b|user\b|<)[a-z_][a-z0-9_.-]*/"), "absolute path into a Linux home directory"),
    (re.compile(r"(?<![\w.:\\/])/Users/(?!Shared\b|<)[A-Za-z][\w.-]*/"), "absolute path into a macOS home directory"),
    (re.compile(r"(?i)discord(app)?\.com/api/webhooks/"), "webhook URL"),
    (re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"), "private key"),
    (re.compile(r"\b(gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|AKIA[0-9A-Z]{16}|xox[abpr]-[A-Za-z0-9-]{10,})"), "access token"),
    (re.compile(r"(?i)\bDESKTOP-[A-Z0-9]{6,}\b"), "Windows machine name"),
]
COMMENT = {
    ".rs": re.compile(r"^\s*(//|/\*|\*/)"),
    ".py": re.compile(r"^\s*#"),
    ".ps1": re.compile(r"^\s*(#|<#|#>)"),
    ".psm1": re.compile(r"^\s*(#|<#|#>)"),
}
COMMENT_OK = re.compile(r"^\s*#Requires\b")


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True)
    if r.returncode != 0:
        sys.exit("git " + " ".join(args) + ": " + r.stderr.decode("utf-8", "replace"))
    return r.stdout


def changed(base, head):
    out = git("diff", "--name-status", "--no-renames", "-z", base, head).decode("utf-8", "replace").split("\0")
    res = []
    for i in range(0, len(out) - 1, 2):
        if out[i] and out[i][0] != "D":
            res.append(out[i + 1])
    return res


def added_lines(base, head, path):
    out = git("diff", "-U0", "--no-color", "--no-ext-diff", base, head, "--", path).decode("utf-8", "replace")
    n = 0
    for line in out.splitlines():
        if line.startswith("@@"):
            m = re.search(r"\+(\d+)", line)
            n = int(m.group(1)) if m else 0
            continue
        if line.startswith("+++"):
            continue
        if line.startswith("+"):
            yield n, line[1:]
            n += 1


def check_files(paths, read):
    errors = []
    for p in paths:
        ext = os.path.splitext(p)[1].lower()
        if ext in BINARY_EXT:
            errors.append((p, "binary or game/console data file type " + ext))
            continue
        if GAME_PATH.search(p):
            errors.append((p, "game or console data path"))
            continue
        data = read(p)
        if data is None:
            continue
        if b"\0" in data[:8192]:
            errors.append((p, "binary content"))
        elif len(data) > MAX_BYTES and not LARGE_OK.search(p):
            errors.append((p, "file is %d bytes (limit %d)" % (len(data), MAX_BYTES)))
    return errors


def check_lines(base, head, paths):
    errors = []
    for p in paths:
        ext = os.path.splitext(p)[1].lower()
        rule = COMMENT.get(ext)
        for n, text in added_lines(base, head, p):
            for rx, why in PRIVATE:
                if rx.search(text):
                    errors.append(("%s:%d" % (p, n), why))
            if rule and rule.search(text) and not (n == 1 and text.startswith("#!")) and not COMMENT_OK.search(text):
                errors.append(("%s:%d" % (p, n), "comment line (the project keeps code free of comment lines)"))
    return errors


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--tree", action="store_true")
    a = ap.parse_args()
    if a.tree:
        paths = [p for p in git("ls-files", "-z").decode("utf-8", "replace").split("\0") if p]
        errors = check_files(paths, lambda p: open(p, "rb").read() if os.path.isfile(p) else None)
    else:
        if not a.base:
            sys.exit("--base is required")
        paths = changed(a.base, a.head)

        def read(p):
            r = subprocess.run(["git", "show", a.head + ":" + p], capture_output=True)
            return r.stdout if r.returncode == 0 else None

        errors = check_files(paths, read) + check_lines(a.base, a.head, paths)
        if any(p.startswith(".github/") for p in paths):
            print("::warning::this pull request changes .github/; a maintainer must review workflow and gate changes by hand")
    for where, why in errors:
        print("::error::%s: %s" % (where, why))
    print("%d file(s) checked, %d problem(s)" % (len(paths), len(errors)))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
