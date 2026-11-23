import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pr_gate

TOKEN = "ghp_" + "A" * 36


class AddedLinesTests(unittest.TestCase):
    def setUp(self):
        self.old = os.getcwd()
        self.tmp = tempfile.TemporaryDirectory()
        os.chdir(self.tmp.name)
        self.run_git("init", "-q")
        self.run_git("config", "core.autocrlf", "false")
        self.run_git("commit", "-q", "--allow-empty", "-m", "base")

    def tearDown(self):
        os.chdir(self.old)
        self.tmp.cleanup()

    def run_git(self, *args):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args], check=True, capture_output=True)

    def commit(self, name, text):
        with open(name, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        self.run_git("add", name)
        self.run_git("commit", "-q", "-m", "change")

    def test_lines_starting_with_two_plus_signs_are_checked(self):
        self.commit("notes.md", "a\n++ " + TOKEN + "\nb\n")
        lines = list(pr_gate.added_lines("HEAD^1", "HEAD", "notes.md"))
        self.assertEqual(lines, [(1, "a"), (2, "++ " + TOKEN), (3, "b")])
        errors = pr_gate.check_lines("HEAD^1", "HEAD", ["notes.md"])
        self.assertEqual(errors, [("notes.md:2", "access token")])

    def test_file_headers_are_not_added_lines(self):
        self.commit("a.rs", "fn a() {}\n")
        self.assertEqual(list(pr_gate.added_lines("HEAD^1", "HEAD", "a.rs")), [(1, "fn a() {}")])

    def test_line_numbers_follow_each_hunk(self):
        self.commit("b.md", "".join("%d\n" % i for i in range(1, 11)))
        self.commit("b.md", "".join(("+++x\n" if i in (3, 8) else "%d\n" % i) for i in range(1, 11)))
        self.assertEqual(list(pr_gate.added_lines("HEAD^1", "HEAD", "b.md")), [(3, "+++x"), (8, "+++x")])

    def test_rva_constants_in_compat_crates_are_rejected(self):
        os.makedirs("crates/compat/x/src")
        os.makedirs("crates/other/src")
        body = ("const TITLE_RVA: usize = 0x1a2b3c;\n"
                "pub const OFF: u64 = 0x0014_5e90;\n"
                "const SMALL: usize = 0x1f0;\n"
                "const PADDED: u64 = 0x0000_00ff;\n"
                "const WORD: u32 = 0x8007_0057;\n"
                "const SENTINEL: usize = 0x5842_4F58_4F45_4D11;\n"
                "const MASK: u64 = 0xffff_ffff_ff00_0000;\n"
                "const NINE: u64 = 0x1_2345_6789;\n"
                "const SEVEN: usize = 0x0001_2345_67;\n")
        self.commit("crates/compat/x/src/a.rs", body)
        self.commit("crates/other/src/a.rs", body)
        why = "title RVA-style constant (usize/u64 with 6-8 significant hex digits) in a compat crate"
        self.assertEqual(pr_gate.check_lines("HEAD~2", "HEAD", ["crates/compat/x/src/a.rs", "crates/other/src/a.rs"]),
                         [("crates/compat/x/src/a.rs:1", why), ("crates/compat/x/src/a.rs:2", why),
                          ("crates/compat/x/src/a.rs:9", why)])

    def test_added_rust_comment_lines_are_rejected(self):
        self.commit("c.rs", "fn c() {}\n// note\n    //! doc\nlet u = \"http://x\";\n")
        why = "comment line (the project keeps code free of comment lines)"
        self.assertEqual(pr_gate.check_lines("HEAD^1", "HEAD", ["c.rs"]), [("c.rs:2", why), ("c.rs:3", why)])


if __name__ == "__main__":
    unittest.main()
