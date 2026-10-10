#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Checks the static website in website/ without a browser or a network:
#
#   - every relative href/src (and og/twitter image) of website/*.html names
#     a file in website/, and every #fragment names an id on its page;
#   - every link to this repository's files on GitHub (blob/tree/main) names
#     a path that exists in this checkout;
#   - pages load nothing from outside (scripts, stylesheets, images, icons);
#   - every page has a <title>, a meta description, a viewport, and the
#     og:title / og:description / og:image / twitter:card tags;
#   - every file in website/ carries an SPDX-FileCopyrightText line near its
#     top (a PNG in a tEXt chunk);
#   - data.js parses as the strict JSON its header promises, every figure has
#     the schema's fields, its source path exists, its commit exists and its
#     onMain flag is true exactly when the commit is an ancestor of HEAD;
#   - every data-fig / data-prov / data-note / data-tiles / data-status hook
#     the pages use resolves to figures in data.js;
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
from html.parser import HTMLParser
from urllib.parse import urlsplit, unquote

ROOT = sys.argv[1]
SITE = os.path.join(ROOT, "website")
REPO_URL = "https://github.com/signal-slot/yakogl"
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
                self.refs.append((tag, attr, a[attr]))
        if tag == "meta":
            key = a.get("name") or a.get("property")
            if key:
                self.meta[key] = a.get("content") or ""
                if key in ("og:image", "twitter:image"):
                    self.refs.append(("meta", key, a.get("content") or ""))
        if tag == "title":
            self._in_title = True
            self.title = ""
        for h in ("data-fig", "data-prov", "data-note", "data-tiles", "data-status"):
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
        if u.scheme in ("http", "https") or value.startswith("//"):
            external_load = tag in ("script", "img", "iframe", "source") or (tag == "link") or tag == "meta"
            if external_load:
                err(f"{name}: <{tag} {attr}> loads an outside resource: {value}")
            if value.startswith(REPO_URL + "/blob/main/") or value.startswith(REPO_URL + "/tree/main/"):
                path = unquote(u.path.split("/", 5)[5]) if u.path.count("/") >= 5 else ""
                if not os.path.exists(os.path.join(ROOT, path)):
                    err(f"{name}: GitHub link to a path that does not exist here: {path}")
            continue
        if u.scheme in ("mailto", "tel", "data", "javascript"):
            if u.scheme == "javascript":
                err(f"{name}: javascript: URL")
            continue
        target = name if u.path == "" else os.path.normpath(os.path.join(os.path.dirname(name), unquote(u.path)))
        if target.startswith(".."):
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
    if have_git and git("cat-file", "-e", data.get("checkedAgainst", {}).get("commit", "") + "^{commit}").returncode:
        err("data.js: checkedAgainst.commit is not a commit here")
    REQ = ("label", "board", "value", "unit", "commit", "onMain", "date", "source", "section", "status", "note")
    for fid, f in figs.items():
        miss = [k for k in REQ if k not in f]
        if miss:
            err(f"data.js: {fid}: missing {', '.join(miss)}")
            continue
        if f["board"] is not None and f["board"] not in boards:
            err(f"data.js: {fid}: board {f['board']} unknown")
        if f["status"] not in ("pass", "warn", "fail", "info"):
            err(f"data.js: {fid}: status {f['status']}")
        if f["date"] is not None and not re.fullmatch(r"\d{4}(-\d{2}(-\d{2})?)?", f["date"]):
            err(f"data.js: {fid}: date {f['date']!r}")
        if (f["date"] is None or f["commit"] is None) and not f["note"]:
            err(f"data.js: {fid}: no date or commit, and no note saying why")
        v = f["value"]
        if isinstance(v, dict):
            for k in ("pass", "fail"):
                if k not in v:
                    err(f"data.js: {fid}: value has no {k}")
        elif not isinstance(v, (int, float, str)):
            err(f"data.js: {fid}: value is {type(v).__name__}")
        if not os.path.isfile(os.path.join(ROOT, f["source"])):
            err(f"data.js: {fid}: source {f['source']} does not exist")
        c = f["commit"]
        if c is not None and have_git:
            if not re.fullmatch(r"[0-9a-f]{7,40}", c) or git("cat-file", "-e", c + "^{commit}").returncode:
                err(f"data.js: {fid}: commit {c} is not a commit in this repository")
            else:
                on_main = git("merge-base", "--is-ancestor", c, "HEAD").returncode == 0
                if on_main != bool(f["onMain"]):
                    err(f"data.js: {fid}: onMain is {f['onMain']} but {c} is {'' if on_main else 'not '}an ancestor of HEAD")
        elif c is None and f["onMain"]:
            err(f"data.js: {fid}: onMain true without a commit")
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
