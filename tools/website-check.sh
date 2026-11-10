#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Checks the static website in website/ without a browser or a network:
#
#   - every local href/src (including canonical URLs and og/twitter images) names
#     a file in website/, and every #fragment names an id on its page;
#   - every link to this repository's files on GitHub (blob/tree/main) names
#     a path that exists in this checkout;
#   - pages load nothing from outside (scripts, stylesheets, images, icons);
#   - every page has a <title>, a meta description, a viewport, and the
#     og:title / og:description / og:image / twitter:card tags;
#   - every file in website/ carries an SPDX-FileCopyrightText line near its
#     top (a PNG in a tEXt chunk);
#   - data.js parses as the strict JSON its header promises, every figure has
#     the schema's fields and a public source snapshot. Public build commits
#     exist and onMain is true exactly for ancestors of HEAD. Unpublished
#     build hashes must occur in their cited first-public-commit record;
#   - every data-fig / data-prov / data-note / data-tiles / data-status /
#     data-summary hook the pages use resolves to figures in data.js;
#   - the glmark2.<board> figures (value, commit, date) equal README.md's
#     glmark2 table ('| Board | Score |'), and the deqp2.<board> counts (fail,
#     pass, notSupported, warnings) equal its dEQP-GLES2 table ('| Board |
#     Failures | Notes |'); a missing or unparsable README row also fails
#     (deqp2.am62p.ms4 has no README row and is not compared);
#   - no placeholder text from the drafts is left (TBD, photo or screenshot
#     slots, "Draft site") and no home or /tmp path.
#
# Usage: tools/website-check.sh            (from anywhere in the repository)
# Exit status 0 when every check passes; each failure is printed.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 -I - "$ROOT" <<'PY'
import json
import os
import re
import subprocess
import sys
from datetime import date
from html.parser import HTMLParser
from urllib.parse import urlsplit, unquote

ROOT = sys.argv[1]
SITE = os.path.join(ROOT, "website")
REPO_URL = "https://github.com/signal-slot/yakogl"
SITE_ORIGIN = "https://yako.gl"
FIRST_PUBLIC_COMMIT = "381b0f09e9c8e7b3d437946c888224f12a27b367"
FIRST_PUBLIC_DATE = "2026-10-27"
# Closed set of measurements verified in the first public snapshot. A new
# result should name a public repository commit, not widen this exception.
PRE_PUBLICATION_BUILDS = {
    "deqp2.am62p": ("996bc35f", "2026-10-26", "tools/cts/RESULTS.md", "ES 2.0: dEQP-GLES2 (stage 1, met)"),
    "deqp2.am62p.ms4": ("e9901869a", "2026-10-15", "tools/cts/RESULTS.md", "ES 2.0: dEQP-GLES2 (stage 1, met)"),
    "deqp2.imx8mp": ("d0c5dc48", "2026-10-23", "tools/cts/RESULTS.md", "ES 2.0: dEQP-GLES2 (stage 1, met)"),
    "deqp2.pi4": ("46f93767", "2026-10-24", "tools/cts/RESULTS.md", "ES 2.0: dEQP-GLES2 (stage 1, met)"),
    "deqp3.am62p": ("ad2b5ed", "2026-10-14", "tools/cts/RESULTS.md", "ES 3.0: dEQP-GLES3 (stage 4)"),
    "gltest.am62p": ("445c5653", "2026-10-20", "tools/cts/RESULTS.md", "Common set (stage 2 input)"),
    "gltest.imx8mp": ("445c5653", "2026-10-20", "tools/cts/RESULTS.md", "Common set (stage 2 input)"),
    "gltest.pi4": ("445c5653", "2026-10-20", "tools/cts/RESULTS.md", "Common set (stage 2 input)"),
    "glmark2.imx8mp": ("445c5653", "2026-10-20", "tools/cts/RESULTS.md", "Common set (stage 2 input)"),
    "glmark2.pi4": ("445c5653", "2026-10-20", "tools/cts/RESULTS.md", "Common set (stage 2 input)"),
}
errors = []


def err(msg):
    if msg not in errors:
        errors.append(msg)


class Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.refs = []      # (tag, attr, value)
        self.ids = set()
        self.meta = {}
        self.title = None
        self._in_title = False
        self.hooks = []     # (attr, value)

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "id" in a:
            self.ids.add(a["id"])
        for attr in ("href", "src"):
            if attr in a and a[attr] is not None:
                ref_tag = "canonical" if tag == "link" and a.get("rel") == "canonical" else tag
                self.refs.append((ref_tag, attr, a[attr]))
        if tag == "meta":
            key = a.get("name") or a.get("property")
            if key:
                self.meta[key] = a.get("content") or ""
                if key in ("og:image", "twitter:image"):
                    self.refs.append(("meta", key, a.get("content") or ""))
                elif key == "og:url":
                    self.refs.append(("meta-url", key, a.get("content") or ""))
        if tag == "title":
            self._in_title = True
            self.title = ""
        for h in ("data-fig", "data-prov", "data-note", "data-tiles", "data-status", "data-summary"):
            if h in a:
                self.hooks.append((h, a[h]))

    def handle_endtag(self, tag):
        if tag == "title":
            self._in_title = False

    def handle_data(self, data):
        if self._in_title:
            self.title += data


def git(*args):
    return subprocess.run(["git", "-C", ROOT] + list(args), capture_output=True, text=True)


# ---- pages -------------------------------------------------------------
pages = {}
for name in sorted(os.listdir(SITE)):
    if name.endswith(".html"):
        p = Page()
        with open(os.path.join(SITE, name), encoding="utf-8") as f:
            p.feed(f.read())
        pages[name] = p
if not pages:
    err("website/: no .html pages")

nrefs = 0
for name, p in pages.items():
    if not (p.title or "").strip():
        err(f"{name}: no <title>")
    for key in ("description", "viewport", "og:title", "og:description", "og:image", "twitter:card"):
        if not p.meta.get(key, "").strip():
            err(f"{name}: no meta {key}")
    for tag, attr, value in p.refs:
        nrefs += 1
        u = urlsplit(value)
        same_site = u.scheme + "://" + u.netloc == SITE_ORIGIN
        if u.scheme in ("http", "https") or value.startswith("//"):
            if same_site:
                u = u._replace(scheme="", netloc="", path=u.path.lstrip("/") or "index.html")
            else:
                external_load = tag in ("script", "img", "iframe", "source", "link", "meta")
                if external_load:
                    err(f"{name}: <{tag} {attr}> loads an outside resource: {value}")
                if tag in ("canonical", "meta-url"):
                    err(f"{name}: {tag} must use {SITE_ORIGIN}: {value}")
                if value.startswith(REPO_URL + "/blob/main/") or value.startswith(REPO_URL + "/tree/main/"):
                    path = unquote(u.path.split("/", 5)[5]) if u.path.count("/") >= 5 else ""
                    if not os.path.exists(os.path.join(ROOT, path)):
                        err(f"{name}: GitHub link to a path that does not exist here: {path}")
                continue
        if u.scheme in ("mailto", "tel", "data", "javascript"):
            if u.scheme == "javascript":
                err(f"{name}: javascript: URL")
            continue
        if tag in ("canonical", "meta-url") and not same_site:
            err(f"{name}: {tag} must be an absolute {SITE_ORIGIN} URL: {value}")
        decoded_path = unquote(u.path)
        base_dir = "" if same_site or decoded_path.startswith("/") else os.path.dirname(name)
        target = name if decoded_path == "" else os.path.normpath(os.path.join(base_dir, decoded_path.lstrip("/")))
        resolved = os.path.realpath(os.path.join(SITE, target))
        if os.path.commonpath((os.path.realpath(SITE), resolved)) != os.path.realpath(SITE):
            err(f"{name}: {value} leaves website/")
            continue
        if not os.path.isfile(os.path.join(SITE, target)):
            err(f"{name}: {attr}={value}: no file website/{target}")
            continue
        if u.fragment and target.endswith(".html"):
            ids = pages[target].ids if target in pages else set()
            if u.fragment not in ids:
                err(f"{name}: {value}: no id '{u.fragment}' in {target}")

# ---- SPDX --------------------------------------------------------------
for dirpath, _, files in os.walk(SITE):
    for fn in files:
        path = os.path.join(dirpath, fn)
        rel = os.path.relpath(path, ROOT)
        with open(path, "rb") as f:
            data = f.read()
        if fn.endswith(".png"):
            ok = b"tEXtSPDX-FileCopyrightText\x00" in data
        else:
            ok = b"SPDX-FileCopyrightText:" in b"\n".join(data.splitlines()[:6])
        if not ok:
            err(f"{rel}: no SPDX-FileCopyrightText near the top")

# ---- leftovers ---------------------------------------------------------
BAD = [r"\[TBD\]", r"\bTBD\b", r"Screenshot slot", r"Video / photo slot", r"Draft site",
       r"/home/", r"(?<![\w.])/tmp/", r"example\.invalid"]
for dirpath, _, files in os.walk(SITE):
    for fn in files:
        if fn.endswith(".png"):
            continue
        path = os.path.join(dirpath, fn)
        with open(path, encoding="utf-8") as f:
            text = f.read()
        for pat in BAD:
            for m in re.finditer(pat, text):
                line = text.count("\n", 0, m.start()) + 1
                err(f"{os.path.relpath(path, ROOT)}:{line}: leftover '{m.group(0)}'")

# ---- data.js -----------------------------------------------------------
data = None
dpath = os.path.join(SITE, "data.js")
try:
    with open(dpath, encoding="utf-8") as f:
        src = f.read()
    body = "\n".join(l for l in src.splitlines() if not l.lstrip().startswith("//"))
    m = re.match(r"\s*window\.YAKOGL_DATA\s*=\s*(.*?);?\s*$", body, re.S)
    if not m:
        raise ValueError("no 'window.YAKOGL_DATA = {...};' assignment")
    data = json.loads(m.group(1))
except (OSError, ValueError) as e:
    err(f"website/data.js: does not parse: {e}")

if data is not None:
    boards = {b.get("id") for b in data.get("boards", [])}
    if boards != {"am62p", "imx8mp", "pi4"}:
        err(f"data.js: boards are {sorted(boards)}")
    figs = data.get("figures", {})
    have_git = git("rev-parse", "HEAD").returncode == 0
    if not have_git:
        err("data.js: a Git checkout is required to verify public-record provenance")

    def public_commit(c):
        return (isinstance(c, str) and re.fullmatch(r"[0-9a-f]{40}", c)
                and have_git and git("cat-file", "-e", c + "^{commit}").returncode == 0
                and git("merge-base", "--is-ancestor", c, "HEAD").returncode == 0)

    if have_git:
        boundary = git("rev-list", "--parents", "-n", "1", FIRST_PUBLIC_COMMIT)
        if boundary.returncode or boundary.stdout.strip() != FIRST_PUBLIC_COMMIT:
            err("data.js: the first-public-commit boundary is not the repository root")
        if git("show", "-s", "--format=%cs", FIRST_PUBLIC_COMMIT).stdout.strip() != FIRST_PUBLIC_DATE:
            err("data.js: the first-public-commit date does not match")

    checked = data.get("checkedAgainst", {})
    if not public_commit(checked.get("commit")):
        err("data.js: checkedAgainst.commit must be a full public ancestor commit")
    try:
        date.fromisoformat(checked.get("date", ""))
    except (TypeError, ValueError):
        err("data.js: checkedAgainst.date must be YYYY-MM-DD")

    record_cache = {}

    def read_record(commit, path):
        key = (commit, path)
        if key not in record_cache:
            r = git("show", commit + ":" + path)
            record_cache[key] = r.stdout if r.returncode == 0 else None
        return record_cache[key]

    def markdown_section(text, title):
        headings = list(re.finditer(r"^(#{1,6})[ \t]+(.+?)[ \t]*$", text, re.M))
        for i, heading in enumerate(headings):
            if heading.group(2) == title:
                stop = next((h.start() for h in headings[i + 1:]
                             if len(h.group(1)) <= len(heading.group(1))), len(text))
                return text[heading.end():stop]
        return None

    REQ = ("label", "board", "value", "unit", "commit", "onMain", "date", "source", "section", "status", "note", "provenance")
    for fid, f in figs.items():
        if not isinstance(f, dict):
            err(f"data.js: {fid}: figure must be an object")
            continue
        miss = [k for k in REQ if k not in f]
        if miss:
            err(f"data.js: {fid}: missing {', '.join(miss)}")
            continue
        if f["board"] is not None and f["board"] not in boards:
            err(f"data.js: {fid}: board {f['board']} unknown")
        if f["status"] not in ("pass", "warn", "fail", "info"):
            err(f"data.js: {fid}: status {f['status']}")
        if type(f["onMain"]) is not bool:
            err(f"data.js: {fid}: onMain must be a boolean")
        measured = f["date"]
        if measured is not None:
            if not isinstance(measured, str) or not re.fullmatch(r"\d{4}(-\d{2}(-\d{2})?)?", measured):
                err(f"data.js: {fid}: date {measured!r}")
            else:
                try:
                    date.fromisoformat(measured + ("-01-01" if len(measured) == 4 else "-01" if len(measured) == 7 else ""))
                except ValueError:
                    err(f"data.js: {fid}: invalid date {measured!r}")
        if not isinstance(f["note"], str):
            err(f"data.js: {fid}: note must be text")
        elif (measured is None or f["commit"] is None) and not f["note"].strip():
            err(f"data.js: {fid}: no date or commit, and no note saying why")
        v = f["value"]
        if isinstance(v, dict):
            for k in ("pass", "fail"):
                if k not in v:
                    err(f"data.js: {fid}: value has no {k}")
            counters = ("pass", "fail", "notSupported", "warnings", "noResult", "total")
            if any(type(v[k]) is not int or v[k] < 0 for k in counters if k in v):
                err(f"data.js: {fid}: result counts must be nonnegative integers")
            elif "total" in v and sum(v.get(k, 0) for k in counters[:-1]) != v["total"]:
                err(f"data.js: {fid}: result counts do not add up to total")
        elif type(v) not in (int, float, str):
            err(f"data.js: {fid}: value is {type(v).__name__}")

        source = f["source"]
        safe_source = (isinstance(source, str) and source and not os.path.isabs(source)
                       and os.path.normpath(source) == source and source != "."
                       and not source.startswith("../") and ":" not in source
                       and "\\" not in source)
        if not safe_source or not os.path.isfile(os.path.join(ROOT, source)):
            err(f"data.js: {fid}: source {source!r} is not a repository file")

        provenance = f["provenance"]
        if not isinstance(provenance, dict):
            err(f"data.js: {fid}: provenance must be an object")
            continue
        kind = provenance.get("kind")
        record_commit = provenance.get("recordCommit")
        valid_record = public_commit(record_commit)
        if not valid_record:
            err(f"data.js: {fid}: recordCommit must be a full public ancestor commit")
        elif public_commit(checked.get("commit")) and git("merge-base", "--is-ancestor", record_commit, checked["commit"]).returncode:
            err(f"data.js: {fid}: recordCommit is newer than checkedAgainst.commit")
        record = read_record(record_commit, source) if valid_record and safe_source else None
        if valid_record and safe_source and record is None:
            err(f"data.js: {fid}: source is absent at recordCommit")

        c = f["commit"]
        valid_hash = isinstance(c, str) and re.fullmatch(r"[0-9a-f]{7,40}", c)
        if kind == "pre-publication":
            # This is a closed historical exception, not permission to accept
            # arbitrary missing commits. The hash must already be in the cited
            # file of the immutable first public snapshot, with an earlier date.
            if record_commit != FIRST_PUBLIC_COMMIT:
                err(f"data.js: {fid}: pre-publication record must use the first public snapshot")
            if (c, measured, source, f["section"]) != PRE_PUBLICATION_BUILDS.get(fid):
                err(f"data.js: {fid}: pre-publication measurement is not in the verified registry")
            section = markdown_section(record, f["section"]) if record is not None else None
            if not valid_hash:
                err(f"data.js: {fid}: pre-publication build needs a commit identifier")
            elif have_git:
                if git("merge-base", "--is-ancestor", c, "HEAD").returncode == 0:
                    err(f"data.js: {fid}: public commit must use repository provenance")
                if section is not None and not re.search(r"(?<![0-9a-f])" + re.escape(c) + r"(?![0-9a-f])", section):
                    err(f"data.js: {fid}: pre-publication hash is absent from its cited section")
            if f["onMain"] is not False:
                err(f"data.js: {fid}: pre-publication build must have onMain false")
            if not isinstance(measured, str) or len(measured) != 10 or measured >= FIRST_PUBLIC_DATE:
                err(f"data.js: {fid}: pre-publication measurement must predate {FIRST_PUBLIC_DATE}")
            if record is not None and section is None:
                err(f"data.js: {fid}: pre-publication section is absent from its public record")
        elif kind == "repository":
            if not valid_hash or not have_git or git("cat-file", "-e", c + "^{commit}").returncode:
                err(f"data.js: {fid}: commit {c} is not a commit in this repository")
            else:
                on_main = git("merge-base", "--is-ancestor", c, "HEAD").returncode == 0
                if on_main != f["onMain"]:
                    err(f"data.js: {fid}: onMain is {f['onMain']} but {c} is {'' if on_main else 'not '}an ancestor of HEAD")
        elif kind in ("external", "unrecorded-build"):
            if c is not None or f["onMain"] is not False:
                err(f"data.js: {fid}: {kind} needs commit null and onMain false")
            if kind == "unrecorded-build" and f.get("unverified") is not True:
                err(f"data.js: {fid}: unrecorded build must be marked unverified")
        else:
            err(f"data.js: {fid}: unknown provenance kind {kind!r}")
    for name, p in pages.items():
        for hook, val in p.hooks:
            if hook in ("data-fig", "data-prov", "data-note"):
                if val not in figs:
                    err(f"{name}: {hook}=\"{val}\" is not a figure in data.js")
            elif hook == "data-status":
                if val not in boards:
                    err(f"{name}: data-status=\"{val}\" is not a board")
            elif hook == "data-tiles":
                if not any(fid == f"{val}.{b}" for fid in figs for b in boards):
                    err(f"{name}: data-tiles=\"{val}\" matches no figure")
            elif hook == "data-summary":
                # site.js fills one row per board from these three figures.
                if val != "boards":
                    err(f"{name}: data-summary=\"{val}\" is not \"boards\"")
                for b in sorted(boards):
                    for kind in ("deqp2", "glmark2", "boot"):
                        if f"{kind}.{b}" not in figs:
                            err(f"{name}: data-summary needs the figure {kind}.{b}")

    # ---- README.md Status tables ---------------------------------------
    # The glmark2 and dEQP-GLES2 figures follow README's Status tables. Of a
    # glmark2 score cell the first number, the first `commit` and the first
    # date are the current figure; older runs follow it in the same cell.
    README_BOARDS = {"AM62P": "am62p", "Raspberry Pi 4": "pi4", "i.MX8MP": "imx8mp"}
    try:
        with open(os.path.join(ROOT, "README.md"), encoding="utf-8") as f:
            readme = f.read()
    except OSError as e:
        readme = None
        err(f"README.md: cannot be read: {e}")

    def readme_table(header):
        # Rows of the table whose header line is exactly `header`, as cell lists.
        lines = readme.splitlines()
        for i, line in enumerate(lines):
            if line.strip() == header:
                rows = []
                for row in lines[i + 2:]:
                    if not row.lstrip().startswith("|"):
                        break
                    rows.append([c.strip() for c in row.strip().strip("|").split("|")])
                return rows
        err(f"README.md: no table with the header '{header}'")
        return None

    def count(text):
        return int(text.replace(",", ""))

    def compare(fid, field, have, want):
        if fid not in figs or not isinstance(figs[fid], dict):
            err(f"data.js: {fid}: missing, but README.md has a figure for it")
        elif have != want:
            err(f"data.js: {fid}: {field} is {have!r}, README.md has {want!r}")

    if readme is not None:
        rows = readme_table("| Board | Score |")
        if rows is not None:
            seen = set()
            for cells in rows:
                board = README_BOARDS.get(cells[0])
                m = re.match(r"(\d[\d,]*)\b.*?`([0-9a-f]{7,40})`.*?(\d{4}-\d{2}-\d{2})", cells[1]) if len(cells) == 2 else None
                if board is None or m is None:
                    err(f"README.md: glmark2 row cannot be parsed: | {' | '.join(cells)} |")
                    continue
                seen.add(board)
                fid = f"glmark2.{board}"
                f = figs.get(fid) if isinstance(figs.get(fid), dict) else {}
                compare(fid, "value", f.get("value"), count(m.group(1)))
                c = f.get("commit")
                if fid in figs and not (isinstance(c, str) and len(c) >= 7
                                        and (c.startswith(m.group(2)) or m.group(2).startswith(c))):
                    err(f"data.js: {fid}: commit is {c!r}, README.md has {m.group(2)!r}")
                compare(fid, "date", f.get("date"), m.group(3))
            for board in sorted(set(README_BOARDS.values()) - seen):
                err(f"README.md: glmark2 table has no row for {board}")
        rows = readme_table("| Board | Failures | Notes |")
        if rows is not None:
            seen = set()
            for cells in rows:
                board = README_BOARDS.get(cells[0])
                if board is None or len(cells) != 3 or not re.fullmatch(r"\d[\d,]*", cells[1]):
                    err(f"README.md: dEQP-GLES2 row cannot be parsed: | {' | '.join(cells)} |")
                    continue
                want = {"fail": count(cells[1])}
                for field, pat in (("pass", r"(\d[\d,]*)(?: of \d[\d,]* cases begun)? pass\b"),
                                   ("notSupported", r"(\d[\d,]*) not supported\b"),
                                   ("warnings", r"(\d[\d,]*) (?:quality )?warnings\b")):
                    m = re.search(pat, cells[2])
                    if m is None:
                        err(f"README.md: dEQP-GLES2 row {cells[0]}: no {field} count in the notes")
                    else:
                        want[field] = count(m.group(1))
                seen.add(board)
                fid = f"deqp2.{board}"
                v = figs.get(fid, {}).get("value") if isinstance(figs.get(fid), dict) else None
                for field, n in want.items():
                    compare(fid, f"value.{field}", v.get(field) if isinstance(v, dict) else None, n)
            for board in sorted(set(README_BOARDS.values()) - seen):
                err(f"README.md: dEQP-GLES2 table has no row for {board}")
    nfig = len(figs)
else:
    nfig = 0

if errors:
    for e in errors:
        print("FAIL", e)
    print(f"website-check: {len(errors)} problem(s)")
    sys.exit(1)
print(f"website-check: ok ({len(pages)} pages, {nrefs} links and sources, {nfig} figures)")
PY
