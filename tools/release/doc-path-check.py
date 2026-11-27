#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Path check of a document's backticked spans (checklist lines 1 and 5).

    python3 -I tools/release/doc-path-check.py FILE [--section HEADING] [--rev REV]
    python3 -I tools/release/doc-path-check.py --self-test

Reads FILE from the working tree (or, with --section, only the part from the
line HEADING, for example '## [Unreleased]', down to the next heading of the
same or a higher level) and takes every backticked span outside fenced code
blocks. Each span becomes one or more candidates: brace lists are expanded
(`a/{b,c}.rs` gives `a/b.rs` and `a/c.rs`), a leading `./` is dropped, and so
is a trailing `:LINE` or `:LINE-LINE`. A span with spaces is a command and is
not a path, unless its first word is a path (it holds a `/` or ends in a file
extension), which is then checked alone.

Each candidate is resolved against the files tracked at REV (default HEAD,
`git ls-tree -r`), in this order:

  resolved  an exact tracked file; a tracked directory (some tracked path is
            below it); a bare file name (no `/`) that is the name of exactly
            one tracked file; a shell pattern with a `/` (`*`, `?`, `[`) or a run-record
            directory under tools/cts/log/ with a `<...>` placeholder that at
            least one tracked path matches.
  non-path  not resolved, and one of: no `/` and no file extension (a word, a
            number, an option, a bare directory name); a bare extension such
            as `.a`; an all-caps identifier or prefix such as `GLES_PRIM_`
            (several joined by `/` too); a `<...>` placeholder (outside
            tools/cts/log/); a URL; a command (above); a name in NON_PATHS.
  missing   everything else: a path that is not tracked, a bare name that
            matches no tracked file or several (named with their count), a
            pattern that matches nothing.

Output (stdout): one line per distinct candidate, `resolved`, `non-path` or
`missing`, its first line number in FILE, the candidate and how it was read;
then `summary: N span(s), C candidate(s): R resolved, P non-path, M missing`.
Exit 0 with no missing candidate, 1 with at least one, 2 when the check
could not run (no such file or section, not a git work tree, bad REV).

Standard library only, and it runs under `python3 -I`. It reads the
repository and FILE and writes nothing.
"""

import argparse
import fnmatch
import os
import re
import subprocess
import sys

# Names that have a file extension but are not tracked files of the tree:
# build outputs and console captures a document names as what a step makes or
# reads, and API names that hold a `/`. Each entry is one exact candidate.
# Add a name here only when it is not, and never will be, a tracked file.
NON_PATHS = {
    "libyakogl.a": "build output (the static library the cmake and make builds write)",
    "uart.log": "console capture a board session writes, never committed",
    "glTexSubImage2D/3D": "two GL entry point names, not a path",
}

PLACEHOLDER = re.compile(r"<[^<>]+>")
EXTENSION = re.compile(r"\.[A-Za-z][A-Za-z0-9_+-]*$")
BARE_EXTENSION = re.compile(r"^\.[A-Za-z0-9]+$")
ALL_CAPS = re.compile(r"^[A-Z][A-Z0-9_]*(?:/[A-Z][A-Z0-9_]*)*$")
LINE_SUFFIX = re.compile(r"(?::\d+(?:-\d+)?){1,2}$")
URL = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*://")
HEADING = re.compile(r"^(#{1,6})\s")
FENCE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")
SPAN = re.compile(r"(?<!`)(`+)(?!`)(.+?)(?<!`)\1(?!`)", re.S)
LOG_DIR = "tools/cts/log/"


def strip_fences(lines):
    """The lines with those inside fenced code blocks blanked (kept as '')."""
    out, fence = [], None
    for line in lines:
        m = FENCE.match(line)
        if fence is None and m:
            fence = m.group(1)[0] * len(m.group(1))
            out.append("")
        elif fence is not None:
            if m and m.group(1).startswith(fence):
                fence = None
            out.append("")
        else:
            out.append(line)
    return out


def section_lines(lines, heading):
    """(first line number, lines) of the section HEADING, or None."""
    want = heading.strip()
    m = HEADING.match(want + " ")
    if not m:
        raise ValueError(f"--section {heading!r} is not a Markdown heading")
    level = len(m.group(1))
    visible = strip_fences(lines)
    start = None
    for i, line in enumerate(visible):
        if start is None:
            if line.strip() == want:
                start = i
            continue
        h = HEADING.match(line)
        if h and len(h.group(1)) <= level:
            return start + 1, lines[start:i]
    if start is None:
        return None
    return start + 1, lines[start:]


def spans(lines, first_lineno=1):
    """[(line number, span text)] of the backticked spans, in order.

    A span may run over a line break inside a paragraph (Markdown's rule); a
    blank line ends it. Line breaks become spaces, and one leading and one
    trailing space are dropped, as Markdown does.
    """
    out = []
    visible = strip_fences(lines)
    para, para_start = [], None
    for i, line in enumerate(visible + [""]):
        if line.strip():
            if para_start is None:
                para_start = i
            para.append(line)
            continue
        if para:
            text = "\n".join(para)
            for m in SPAN.finditer(text):
                body = m.group(2).replace("\n", " ")
                if len(body) > 2 and body.startswith(" ") and body.endswith(" "):
                    body = body[1:-1]
                lineno = first_lineno + para_start + text.count("\n", 0, m.start())
                out.append((lineno, body))
        para, para_start = [], None
    return out


def expand_braces(s):
    """`a/{b,c}.rs` -> [`a/b.rs`, `a/c.rs`]; nested and repeated lists too."""
    depth, start = 0, None
    for i, ch in enumerate(s):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}" and depth:
            depth -= 1
            if depth == 0:
                body = s[start + 1:i]
                parts, d, last = [], 0, 0
                for j, c in enumerate(body):
                    if c == "{":
                        d += 1
                    elif c == "}":
                        d -= 1
                    elif c == "," and d == 0:
                        parts.append(body[last:j])
                        last = j + 1
                parts.append(body[last:])
                if len(parts) < 2:
                    return [s]
                out = []
                for p in parts:
                    out.extend(expand_braces(s[:start] + p + s[i + 1:]))
                return out
    return [s]


def looks_like_path(word):
    return "/" in word or bool(EXTENSION.search(word))


def candidates(span):
    """[(candidate, note)] of one span; note is '' or why it was cut."""
    text = span.strip()
    note = ""
    if re.search(r"\s", text):
        first = text.split()[0]
        if not looks_like_path(first) or URL.match(first):
            return [(text, "command")]
        text, note = first, "first word of a command"
    out = []
    for c in expand_braces(text):
        if c.startswith("./"):
            c = c[2:]
        c = LINE_SUFFIX.sub("", c)
        out.append((c, note))
    return out


class Tree:
    def __init__(self, paths):
        self.files = set(paths)
        self.dirs = set()
        self.by_name = {}
        for p in self.files:
            parts = p.split("/")
            for k in range(1, len(parts)):
                self.dirs.add("/".join(parts[:k]))
            self.by_name.setdefault(parts[-1], []).append(p)

    def matches(self, pattern):
        """Tracked files, or directories, that the shell pattern matches."""
        pattern = pattern.rstrip("/")
        hits = [p for p in self.files if fnmatch.fnmatchcase(p, pattern)]
        hits += [d for d in self.dirs if fnmatch.fnmatchcase(d, pattern)]
        return sorted(hits)


def classify(cand, cmd_note, tree):
    """('resolved' | 'non-path' | 'missing', how)."""
    c = cand
    if cmd_note == "command":
        return "non-path", "spaces: a command or a phrase"
    if URL.match(c):
        return "non-path", "a URL"
    if c in NON_PATHS:
        return "non-path", NON_PATHS[c]
    bare = c.rstrip("/")
    if PLACEHOLDER.search(c):
        if c.startswith(LOG_DIR):
            pat = PLACEHOLDER.sub("*", bare)
            hits = tree.matches(pat)
            if hits:
                return "resolved", f"run-record pattern, {len(hits)} tracked match(es)"
            return "missing", "run-record pattern, no tracked match"
        return "non-path", "a <...> placeholder"
    if bare and bare in tree.files and not c.endswith("/"):
        return "resolved", "tracked file"
    if bare and bare in tree.dirs:
        return "resolved", "tracked directory"
    if "/" in bare and any(ch in c for ch in "*?["):
        hits = tree.matches(bare)
        if hits:
            return "resolved", f"pattern, {len(hits)} tracked match(es)"
        return "missing", "pattern, no tracked match"
    if "/" not in bare and bare in tree.by_name:
        hits = tree.by_name[bare]
        if len(hits) == 1:
            return "resolved", f"bare name of {hits[0]}"
        return "missing", f"bare name of {len(hits)} tracked files"
    if BARE_EXTENSION.match(c):
        return "non-path", "a bare extension"
    if ALL_CAPS.match(bare):
        return "non-path", "an all-caps identifier or prefix"
    if "/" not in bare and not EXTENSION.search(bare):
        return "non-path", "no '/' and no file extension"
    if "/" not in bare:
        return "missing", "bare name of no tracked file"
    return "missing", "not tracked"


def check(lines, tree, section=None):
    """(first line, records, n_spans); records: (kind, line, candidate, how)."""
    first = 1
    if section:
        got = section_lines(lines, section)
        if got is None:
            raise LookupError(f"no section {section!r}")
        first, lines = got
    found = spans(lines, first)
    seen, records = set(), []
    for lineno, span in found:
        for cand, note in candidates(span):
            key = (cand, note == "command")
            if key in seen:
                continue
            seen.add(key)
            kind, how = classify(cand, note, tree)
            if note and note != "command":
                how = f"{how} ({note})"
            records.append((kind, lineno, cand, how))
    return first, records, len(found)


def tracked(rev):
    out = subprocess.run(["git", "ls-tree", "-r", "-z", "--name-only", rev],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if out.returncode != 0:
        msg = out.stderr.decode("utf-8", "replace").strip()
        raise RuntimeError(f"git ls-tree {rev}: {msg}")
    return [p for p in out.stdout.decode("utf-8", "surrogateescape").split("\0") if p]


def report(name, section, rev_label, first, records, n_spans, out):
    where = f", section {section!r} from line {first}" if section else ""
    out.write(f"doc-path-check: {name}{where}, tracked files of {rev_label}\n")
    width = max([len(r[2]) for r in records] + [1])
    for kind, lineno, cand, how in records:
        out.write(f"{kind:<9} {lineno:>5}  {cand:<{width}}  {how}\n")
    counts = {k: sum(1 for r in records if r[0] == k) for k in ("resolved", "non-path", "missing")}
    out.write(f"summary: {n_spans} span(s), {len(records)} candidate(s): "
              f"{counts['resolved']} resolved, {counts['non-path']} non-path, "
              f"{counts['missing']} missing\n")
    return counts["missing"]


def self_test():
    tree = Tree([
        "Cargo.toml", "build.rs", "CHANGELOG.md", ".gitignore",
        "src/backend.rs", "src/gl/mod.rs", "src/pvr/mod.rs", "src/v3d/mod.rs",
        "src/v3d/texstate71.rs", "src/v3d/cl71.rs",
        "src/shaderc/online/powervr/mod.rs", "src/shaderc/online/mali/mod.rs",
        "tools/cts/RESULTS.md", "tools/cts/host-deqp-run.sh",
        "tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055/qpa.txt",
        "tools/cts/log/2026-11-24-t6-gles2-S2-8ff01055/qpa.txt",
        "third_party/mesa/src/pvr_pipeline_pds.c",
        "a/mod.rs", "b/mod.rs",
    ])
    doc = """# Title

## [Unreleased]

Uses `src/backend.rs`, `./Cargo.toml` and `build.rs`, the directory
`src/gl/`, `src/{pvr,v3d}/mod.rs` and `src/shaderc/online/{powervr,mali}/`.
Lines: `src/backend.rs:12` and `pvr_pipeline_pds.c:846-864`.
Command: `tools/cts/host-deqp-run.sh --compare` and `cargo test -p x`.
Words: `0 Fail`, `17,165`, `--self-test`, `driver`, `0.1.0`, `.a`, `.gitignore`.
Caps: `GLES_PRIM_`, `GLES_PRIM_/CMP_/CULL_`. Holders: `src/<gpu>/`, `<GLES2/gl2.h>`.
Log: `tools/cts/log/2026-11-24-t6-gles2-S<n>-8ff01055/` and
`tools/cts/log/2026-11-24-t6-gles2-X<n>-8ff01055/`.
Pattern: `src/v3d/*71.rs`, `src/v3d/*99.rs`, not one: `glUniform*ui`, `#[repr(C)]`. Names: `libyakogl.a`, `uart.log`.
Missing: `src/gone.rs`, `docs/gone/`, `mod.rs`, `nothing.rs`, `out.bin`.
A span over a line `src/gl/mod.rs
` break and ``double `tick` span``, a URL `https://example.invalid/a.rs`.

```
`src/in_fence.rs`
## [Not a heading]
```

Still in: `src/pvr/mod.rs`.

## [0.1.0]

`src/after.rs`
"""
    lines = doc.split("\n")
    want = {
        "src/backend.rs": ("resolved", "tracked file"),
        "Cargo.toml": ("resolved", "tracked file"),
        "build.rs": ("resolved", "tracked file"),
        "src/gl/": ("resolved", "tracked directory"),
        "src/pvr/mod.rs": ("resolved", "tracked file"),
        "src/v3d/mod.rs": ("resolved", "tracked file"),
        "src/shaderc/online/powervr/": ("resolved", "tracked directory"),
        "src/shaderc/online/mali/": ("resolved", "tracked directory"),
        "pvr_pipeline_pds.c": ("resolved", "bare name of third_party/mesa/src/pvr_pipeline_pds.c"),
        "tools/cts/host-deqp-run.sh": ("resolved", "tracked file (first word of a command)"),
        "cargo test -p x": ("non-path", "spaces: a command or a phrase"),
        "0 Fail": ("non-path", "spaces: a command or a phrase"),
        "17,165": ("non-path", "no '/' and no file extension"),
        "--self-test": ("non-path", "no '/' and no file extension"),
        "driver": ("non-path", "no '/' and no file extension"),
        "0.1.0": ("non-path", "no '/' and no file extension"),
        ".a": ("non-path", "a bare extension"),
        ".gitignore": ("resolved", "tracked file"),
        "GLES_PRIM_": ("non-path", "an all-caps identifier or prefix"),
        "GLES_PRIM_/CMP_/CULL_": ("non-path", "an all-caps identifier or prefix"),
        "src/<gpu>/": ("non-path", "a <...> placeholder"),
        "<GLES2/gl2.h>": ("non-path", "a <...> placeholder"),
        "tools/cts/log/2026-11-24-t6-gles2-S<n>-8ff01055/":
            ("resolved", "run-record pattern, 2 tracked match(es)"),
        "tools/cts/log/2026-11-24-t6-gles2-X<n>-8ff01055/":
            ("missing", "run-record pattern, no tracked match"),
        "src/v3d/*71.rs": ("resolved", "pattern, 2 tracked match(es)"),
        "src/v3d/*99.rs": ("missing", "pattern, no tracked match"),
        "glUniform*ui": ("non-path", "no '/' and no file extension"),
        "#[repr(C)]": ("non-path", "no '/' and no file extension"),
        "libyakogl.a": ("non-path", NON_PATHS["libyakogl.a"]),
        "uart.log": ("non-path", NON_PATHS["uart.log"]),
        "src/gone.rs": ("missing", "not tracked"),
        "docs/gone/": ("missing", "not tracked"),
        "mod.rs": ("missing", "bare name of 7 tracked files"),
        "nothing.rs": ("missing", "bare name of no tracked file"),
        "out.bin": ("missing", "bare name of no tracked file"),
        "src/gl/mod.rs": ("resolved", "tracked file"),
        "double `tick` span": ("non-path", "spaces: a command or a phrase"),
        "https://example.invalid/a.rs": ("non-path", "a URL"),
    }
    fails = 0

    def ok(cond, what):
        nonlocal fails
        print(("ok    " if cond else "FAIL  ") + what)
        if not cond:
            fails += 1

    first, records, n_spans = check(lines, tree, "## [Unreleased]")
    got = {r[2]: (r[0], r[3]) for r in records}
    ok(first == 3, f"the section starts at its heading's line (got {first})")
    for cand, w in want.items():
        ok(got.get(cand) == w, f"{cand!r} reads {w[0]}: {w[1]} (got {got.get(cand)})")
    extra = sorted(set(got) - set(want))
    ok(not extra, f"no candidate beyond the expected ones (extra: {extra})")
    ok(all(r[2] != "src/in_fence.rs" for r in records), "a fenced block holds no span")
    ok(all(r[2] != "src/after.rs" for r in records), "the next heading of the same level ends the section")
    line_of = {r[2]: r[1] for r in records}
    ok(line_of.get("src/backend.rs") == 5, "a span's line number is its line in the file")
    ok(line_of.get("src/pvr/mod.rs") == 6, "a candidate is listed once, at its first line")
    _, whole, _ = check(lines, tree)
    ok(any(r[2] == "src/after.rs" for r in whole), "without --section the whole file is read")
    try:
        check(lines, tree, "## [Nope]")
        ok(False, "a missing section is an error")
    except LookupError:
        ok(True, "a missing section is an error")
    ok(expand_braces("a/{b,{c,d}}.rs") == ["a/b.rs", "a/c.rs", "a/d.rs"], "nested brace lists expand")
    ok(expand_braces("x/{a,b}/{c,d}") == ["x/a/c", "x/a/d", "x/b/c", "x/b/d"], "two brace lists expand")
    ok(expand_braces("{single}") == ["{single}"], "a brace pair without a comma stays")
    import io
    buf = io.StringIO()
    missing = report("doc.md", "## [Unreleased]", "HEAD", first, records, n_spans, buf)
    ok(missing == 7, f"seven missing candidates counted (got {missing})")
    ok(buf.getvalue().rstrip().endswith("7 missing"), "the summary line ends with the missing count")
    if fails:
        print(f"self-test FAILED: {fails} check(s)")
        return 1
    print("self-test passed")
    return 0


def main(argv):
    ap = argparse.ArgumentParser(
        prog="doc-path-check.py",
        description="Resolve a document's backticked spans against the tracked tree.")
    ap.add_argument("file", nargs="?", help="the Markdown file to read")
    ap.add_argument("--section", help="a heading line, e.g. '## [Unreleased]'")
    ap.add_argument("--rev", default="HEAD", help="the commit whose tracked files count (HEAD)")
    ap.add_argument("--self-test", action="store_true", help="run the built-in checks and exit")
    args = ap.parse_args(argv)
    if args.self_test:
        if args.file or args.section:
            print("doc-path-check: --self-test takes no other argument", file=sys.stderr)
            return 2
        return self_test()
    if not args.file:
        ap.print_usage(sys.stderr)
        return 2
    try:
        with open(args.file, encoding="utf-8") as f:
            lines = f.read().split("\n")
        paths = tracked(args.rev)
        rev_label = subprocess.run(["git", "rev-parse", "--short=12", args.rev],
                                   stdout=subprocess.PIPE, check=True).stdout.decode().strip()
        first, records, n_spans = check(lines, Tree(paths), args.section)
    except (OSError, RuntimeError, LookupError, ValueError, subprocess.CalledProcessError) as e:
        print(f"doc-path-check: {e}", file=sys.stderr)
        return 2
    missing = report(os.path.normpath(args.file), args.section, f"{args.rev} ({rev_label})",
                     first, records, n_spans, sys.stdout)
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
