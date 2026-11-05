#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redaction scan of one revision's tree (release checklist line 8).

The classes of docs/release/redaction-audit.md "Part 3", as a scan anyone can
re-run on any commit without a checkout:

    tools/release/redaction-scan.py [REV]       scan REV's tree (default HEAD)
    tools/release/redaction-scan.py --self-test  run the fixtures

The tree of REV is read with `git ls-tree` and `git cat-file` (binary files
skipped, as `git grep -I` does), with Part 3's exclusions: third_party/ and
docs/release/redaction-audit.md.

Classes (severity H or M is a finding, L is information):

  home-path      M  /home/<name>, /Users/<name>
  private-ipv4   H  10/8, 172.16/12, 192.168/16, 100.64/10 (tailnet), 169.254/16,
                    as whole addresses (not section numbers such as 13.10.3.1.2)
  ipv6-local     H  IPv6 link-local (fe80::/10) and ULA (fc00::/7)
  som-serial     M  verdin-(am62p|imx8mp)-<8 hex> host names
  bridge-port    H  serial-bridge ports 3593x (x one digit)
  tailnet-dns    H  <name>.ts.net
  token          H  ghp_, github_pat_, gho_, AKIA, xox?-, AIza, sk- token shapes,
                    PEM private-key headers
  password       H  (password|passwd|passphrase|pwd|secret|api_key)[:=]<value>
  email          M  e-mail addresses (upstream authors: information, by the
                    allow file)
  lab-layout     L  the lab's checkout layout ($HOME/com/github/..., the
                    non-public yakogl-in-rust checkout)
  owner-login    H  private: the owner's login, whole word, any case
  login-at-host  H  private: <login>@<host> for root and the private logins
  lab-host       M  private: lab machine names (runners, laptop, Pi 4
                    resource, bridge hosts), whole words, any case

Private values never live in the tree. They come from the environment
variable YAKOGL_REDACTION_PRIVATE, or from the file named by
YAKOGL_REDACTION_PRIVATE_FILE (which must lie outside the scanned
repository). The format is one `class: value` per line (`;` also separates
entries in the variable; `#` starts a comment):

    owner-login: <the owner's login>
    login: <a lab, farm or vendor-image login>     (repeatable)
    lab-host: <a runner, laptop, resource or bridge host name>  (repeatable)
    public-name: <the owner's public name>  (repeatable; an owner-login hit
                 inside it is a look-alike, as in Part 3)

With neither set, the private classes are reported as "not run", never as
passed. login-at-host still checks root@<host>.

Look-alikes are dropped by tools/release/redaction-scan.allow (the file next
to this script; --allow FILE replaces it). A rule line is

    action | class | path glob | regex | reason

action `drop` (a look-alike, not counted) or `info` (listed as information,
not a finding); class `*` means any class; the glob is fnmatch over the
whole path (`*` matches across `/`); the regex is searched in the line and
drops a hit its match overlaps. Rules list patterns, never
secrets.

Every value is printed masked as in the audit: first character, `***`, the
length, e.g. `t***[6]`; both sides of an `@` are masked separately. A path
that contains a private value is masked the same way.

Exit status:
  0  every class ran and there is no H/M finding
  1  at least one H/M finding (even when private classes did not run)
  2  usage error, a bad allow or private-values file, or a git error
  3  no finding, but the private classes did not run (no private values)
"""

import argparse
import fnmatch
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ALLOW = os.path.join(HERE, "redaction-scan.allow")
EXCLUDE_PREFIXES = ("third_party/",)
EXCLUDE_PATHS = ("docs/release/redaction-audit.md",)
ENV_VALUES = "YAKOGL_REDACTION_PRIVATE"
ENV_FILE = "YAKOGL_REDACTION_PRIVATE_FILE"
PRIVATE_KEYS = ("owner-login", "public-name", "login", "lab-host")
FINDING = ("H", "M")

EXIT_OK, EXIT_FINDING, EXIT_ERROR, EXIT_NOT_RUN = 0, 1, 2, 3


class ScanError(Exception):
    pass


# --- classes ----------------------------------------------------------------

def _octets(m):
    return [int(m.group(i)) for i in range(1, 5)]


def private_ipv4(m):
    a, b, c, d = _octets(m)
    if max(a, b, c, d) > 255:
        return False
    return (a == 10 or (a == 172 and 16 <= b <= 31) or (a == 192 and b == 168)
            or (a == 100 and 64 <= b <= 127) or (a == 169 and b == 254))


def ipv6_local(m):
    # At least two groups after the prefix, or a `::`: "fd00:" in a hex dump
    # or "fe80:" as a label is not an address.
    rest = m.group(0)
    return "::" in rest or rest.count(":") >= 3


def som_serial(m):
    return True


TOKEN_RE = (
    r"ghp_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{22,}|gho_[A-Za-z0-9]{36}"
    r"|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{35}"
    r"|(?<![A-Za-z0-9_-])sk-[A-Za-z0-9_-]{20,}"
    r"|-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED |PGP )?PRIVATE KEY")

# (key, title, severity, regex, flags, extra check or None)
PUBLIC_CLASSES = [
    ("home-path", "Home path", "M",
     r"/(?:home|Users)/[A-Za-z0-9_.-]+", 0, None),
    ("private-ipv4", "Private / tailnet IPv4", "H",
     r"(?<![\d.])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?!\d|\.\d)", 0,
     private_ipv4),
    ("ipv6-local", "IPv6 link-local / ULA", "H",
     r"(?<![0-9A-Fa-f:])(?:fe[89ab][0-9a-f]|f[cd][0-9a-f]{2}):[0-9a-f:]*[0-9a-f]",
     re.I, ipv6_local),
    ("som-serial", "SoM serial in a host name", "M",
     r"verdin-(?:am62p|imx8mp)-[0-9a-fA-F]{8}(?![0-9A-Za-z])", 0, None),
    ("bridge-port", "Serial-bridge port", "H",
     r"(?<!\d)3593\d(?!\d)", 0, None),
    ("tailnet-dns", "Tailnet DNS name", "H",
     r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net(?![A-Za-z0-9-])", 0, None),
    ("token", "Token / key", "H", TOKEN_RE, 0, None),
    ("password", "Password / secret value", "H",
     r"(?:password|passwd|passphrase|pwd|secret|api_?key)\s*[:=]\s*"
     r"[\"']?(?P<v>[^\s\"',;)}\]]+)", re.I, None),
    ("email", "E-mail address", "M",
     r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"
     r"(?![A-Za-z0-9-])", 0, None),
    ("lab-layout", "Lab checkout layout", "L",
     r"(?:\$HOME|\$\{HOME\}|~)/com/github/[A-Za-z0-9_.-]+|yakogl-in-rust", 0,
     None),
]

PRIVATE_TITLES = {
    "owner-login": ("Owner login", "H"),
    "login-at-host": ("Login at a real host", "H"),
    "lab-host": ("Lab machine name", "M"),
}
CLASS_ORDER = [c[0] for c in PUBLIC_CLASSES[:2]] + [
    "ipv6-local", "owner-login", "login-at-host", "lab-host"] + [
    c[0] for c in PUBLIC_CLASSES[3:]]
ALL_CLASSES = {c[0] for c in PUBLIC_CLASSES} | set(PRIVATE_TITLES)


def word_alternation(values):
    vals = sorted({v for v in values if v}, key=len, reverse=True)
    return "|".join(re.escape(v) for v in vals)


def build_classes(private):
    """[(key, title, severity, compiled regex or None, check)]; None = not run."""
    out = {}
    for key, title, sev, rx, flags, check in PUBLIC_CLASSES:
        out[key] = (key, title, sev, re.compile(rx, flags), check)
    owner = private.get("owner-login", [])
    logins = private.get("login", [])
    hosts = private.get("lab-host", [])
    t, s = PRIVATE_TITLES["owner-login"]
    out["owner-login"] = (
        "owner-login", t, s,
        re.compile(r"(?<![A-Za-z0-9])(?:%s)(?![A-Za-z0-9])"
                   % word_alternation(owner), re.I) if owner else None, None)
    t, s = PRIVATE_TITLES["login-at-host"]
    alt = word_alternation(["root"] + owner + logins)
    out["login-at-host"] = (
        "login-at-host", t, s,
        re.compile(r"(?<![A-Za-z0-9._%+-])(?:" + alt +
                   r")@[A-Za-z0-9][A-Za-z0-9.-]*(?![A-Za-z0-9])", re.I), None)
    t, s = PRIVATE_TITLES["lab-host"]
    out["lab-host"] = (
        "lab-host", t, s,
        re.compile(r"(?<![A-Za-z0-9])(?:%s)(?![A-Za-z0-9])"
                   % word_alternation(hosts), re.I) if hosts else None, None)
    return [out[k] for k in CLASS_ORDER]


# --- private values and allow rules -----------------------------------------

def parse_private(text, origin):
    vals = {}
    for n, raw in enumerate(re.split(r"[\n;]", text), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        key, sep, value = line.partition(":")
        key, value = key.strip(), value.strip()
        if not sep or key not in PRIVATE_KEYS or not value:
            # Never echo the entry: it may hold a private value.
            raise ScanError("%s: entry %d is not `class: value` with class one "
                            "of %s" % (origin, n, ", ".join(PRIVATE_KEYS)))
        vals.setdefault(key, []).append(value)
    return vals


def load_private(repo_top):
    """(values, source) from the environment; ({}, None) when neither is set."""
    text = os.environ.get(ENV_VALUES)
    if text:
        return parse_private(text, ENV_VALUES), ENV_VALUES
    path = os.environ.get(ENV_FILE)
    if not path:
        return {}, None
    real = os.path.realpath(path)
    top = os.path.realpath(repo_top)
    if real == top or real.startswith(top + os.sep):
        raise ScanError("%s names a file inside the repository; private values "
                        "must live outside it" % ENV_FILE)
    try:
        with open(real, encoding="utf-8") as f:
            return parse_private(f.read(), ENV_FILE), ENV_FILE
    except OSError as e:
        raise ScanError("%s: cannot read: %s" % (ENV_FILE, e.strerror))


def load_allow(path):
    rules = []
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as e:
        raise ScanError("allow file %s: %s" % (path, e.strerror))
    for n, raw in enumerate(lines, 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        parts = [p.strip() for p in raw.split(" | ")]
        if len(parts) != 5 or parts[0] not in ("drop", "info") or not parts[4]:
            raise ScanError("%s:%d: not `action | class | glob | regex | reason`"
                            " with action drop or info" % (path, n))
        action, cls, glob, rx, reason = parts
        if cls != "*" and cls not in ALL_CLASSES:
            raise ScanError("%s:%d: unknown class %r" % (path, n, cls))
        try:
            crx = re.compile(rx)
        except re.error as e:
            raise ScanError("%s:%d: bad regex: %s" % (path, n, e))
        rules.append((action, cls, glob, crx, reason, n))
    return rules


def allow_rule(rules, cls, path, line, start, end):
    for rule in rules:
        action, rcls, glob, crx, _, _ = rule
        if rcls not in ("*", cls) or not fnmatch.fnmatchcase(path, glob):
            continue
        for m in crx.finditer(line):
            if m.start() < end and start < m.end():
                return rule
    return None


# --- masking ----------------------------------------------------------------

def mask_word(s):
    return "%s***[%d]" % (s[:1], len(s)) if s else s


def mask(value):
    if "@" in value:
        return "@".join(mask_word(p) for p in value.split("@"))
    return mask_word(value)


def mask_path(path, private):
    words = [v for vs in private.values() for v in vs]
    if not words:
        return path
    rx = re.compile(word_alternation(words), re.I)
    return rx.sub(lambda m: mask_word(m.group(0)), path)


# --- git --------------------------------------------------------------------

def git(repo, *args, data=None):
    p = subprocess.run(["git", "-C", repo, *args], input=data,
                       capture_output=True)
    if p.returncode != 0:
        raise ScanError("git %s: %s" % (" ".join(args),
                                         p.stderr.decode(errors="replace").strip()))
    return p.stdout


def tree_blobs(repo, rev):
    """[(path, blob id)] of REV's tree, without the exclusions."""
    out = git(repo, "ls-tree", "-r", "-z", "--full-tree", rev)
    blobs = []
    for entry in out.decode("utf-8", "surrogateescape").split("\0"):
        if not entry:
            continue
        meta, path = entry.split("\t", 1)
        _, typ, oid = meta.split()
        if typ != "blob" or path.startswith(EXCLUDE_PREFIXES) \
                or path in EXCLUDE_PATHS:
            continue
        blobs.append((path, oid))
    return blobs


def read_blobs(repo, oids):
    """Yield the contents of each blob id, in order (one cat-file process)."""
    p = subprocess.Popen(["git", "-C", repo, "cat-file", "--batch"],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        for oid in oids:
            p.stdin.write(oid.encode() + b"\n")
            p.stdin.flush()
            header = p.stdout.readline().split()
            if len(header) != 3:
                raise ScanError("git cat-file: no object %s" % oid)
            size = int(header[2])
            data = p.stdout.read(size)
            p.stdout.read(1)
            yield data
    finally:
        p.stdin.close()
        p.wait()


# --- scan -------------------------------------------------------------------

def scan(repo, rev, rules, private):
    """{class: {"hits": [...], "info": [...], "dropped": n}}, file count."""
    classes = build_classes(private)
    rules = [("drop", "owner-login", "*", re.compile(re.escape(name)),
              "the owner's public name", 0)
             for name in private.get("public-name", [])] + list(rules)
    result = {c[0]: {"hits": [], "info": [], "dropped": 0} for c in classes}
    blobs = tree_blobs(repo, rev)
    for (path, _), data in zip(blobs, read_blobs(repo, [b for _, b in blobs])):
        if b"\0" in data[:8000]:
            continue
        text = data.decode("utf-8", "replace")
        for lineno, line in enumerate(text.splitlines(), 1):
            for key, _, _, rx, check in classes:
                if rx is None:
                    continue
                for m in rx.finditer(line):
                    if check and not check(m):
                        continue
                    grp = "v" if "v" in rx.groupindex and m.group("v") else 0
                    rule = allow_rule(rules, key, path, line, *m.span())
                    if rule and rule[0] == "drop":
                        result[key]["dropped"] += 1
                        continue
                    hit = (path, lineno, m.group(grp), rule[4] if rule else None)
                    result[key]["info" if rule else "hits"].append(hit)
    return result, classes, len(blobs)


def report(rev_label, nfiles, result, classes, private, source, out):
    """Print the report; return the exit status."""
    findings = not_run = infos = dropped = 0
    print("redaction-scan: %s, %d files (third_party/ and %s excluded)"
          % (rev_label, nfiles, ", ".join(EXCLUDE_PATHS)), file=out)
    print("private values: %s" % (
        "from %s (%s)" % (source, ", ".join(
            "%s x%d" % (k, len(private[k])) for k in PRIVATE_KEYS if k in private))
        if source else "none (set %s or %s)" % (ENV_VALUES, ENV_FILE)), file=out)
    for key, title, sev, rx, _ in classes:
        r = result[key]
        dropped += r["dropped"]
        infos += len(r["info"])
        partial = key == "login-at-host" and not private
        if rx is None or partial:
            not_run += 1
        if rx is None:
            state = "not run (no private values)"
        else:
            n = len(r["hits"])
            if sev in FINDING:
                findings += n
                state = "%d finding(s)" % n if n else "ok"
            else:
                infos += n
                state = "%d informational" % n
            if partial:
                state += "; not run for the private logins (root only)"
        print("[%s] %-14s %s: %s%s" % (sev, key, title, state,
              ", %d look-alike(s) dropped" % r["dropped"] if r["dropped"] else ""),
              file=out)
        for path, lineno, value, _ in r["hits"]:
            print("      %s:%d  %s" % (mask_path(path, private), lineno,
                                       mask(value)), file=out)
        for path, lineno, value, reason in r["info"]:
            print("      info %s:%d  %s  (%s)" % (mask_path(path, private),
                                                  lineno, mask(value), reason),
                  file=out)
    status = (EXIT_FINDING if findings else EXIT_NOT_RUN if not_run else EXIT_OK)
    print("summary: %d H/M finding(s), %d class(es) not run, %d informational, "
          "%d look-alike(s) dropped: %s" % (
              findings, not_run, infos, dropped,
              {EXIT_OK: "clean", EXIT_FINDING: "FINDINGS",
               EXIT_NOT_RUN: "NOT COMPLETE (private classes not run)"}[status]),
          file=out)
    return status


def run(rev, allow_path, out=sys.stdout, repo="."):
    try:
        top = git(repo, "rev-parse", "--show-toplevel").decode().strip()
        sha = git(top, "rev-parse", "--verify", "--quiet",
                  rev + "^{commit}").decode().strip()
        rules = load_allow(allow_path)
        private, source = load_private(top)
        result, classes, nfiles = scan(top, sha, rules, private)
    except ScanError as e:
        print("redaction-scan: %s" % e, file=sys.stderr)
        return EXIT_ERROR
    return report("%s (%s)" % (rev, sha[:12]), nfiles, result, classes,
                  private, source, out)


# --- self-test --------------------------------------------------------------

def self_test():
    """Scan a throwaway repository of fixtures; 0 when every check holds.

    The fixture values are assembled at run time so that this file itself
    holds none of them (the scan reads this file too).
    """
    j = "".join
    dot = ".".join
    priv4 = dot(["10", "0", "0", "1"])
    tail4 = dot(["100", "64", "0", "7"])
    names = {"owner": j(["ownr", "fixture"]),
             "runner": j(["runner-alpha", ".example.invalid"]),
             "login": j(["labuser", "x"]),
             "public": j(["Ownr", "fixture Example"])}
    fixtures = {
        "src/a.txt": "\n".join([
            "docs: 192.0.2.1 198.51.100.2 203.0.113.3 are RFC 5737",
            "section 13.10.3.1.2 is not an address",
            "a private address " + priv4 + " here",
            "tailnet " + tail4 + ":40001",
            "link-local " + j(["fe80", "::1"]) + " and ULA " + j(["fd12", ":3456:789a::1"]),
            "home " + j(["/ho", "me/someone/x"]) + " and " + j(["/Us", "ers/other"]),
            "placeholder /home/<user> is fine",
            "board " + j(["verdin-am62p-", "1a2b3c4d"]) + " and verdin-imx8mp-00000000",
            "bridge port " + j(["3593", "4"]),
            "dns " + j(["box.tail", "0000.ts", ".net"]),
            "token " + j(["gh", "p_"]) + "A" * 36,
            "key " + j(["-----BEGIN ", "RSA PRIVATE KEY-----"]),
            j(["pass", "word = "]) + "hunter2x",
            "sudoers: ALL=(ALL) " + j(["NOPASS", "WD:"]) + " ALL",
            "mail " + j(["someone", "@corp.example.com"]) + " and t@example.invalid",
            "upstream " + j(["dev", "@upstream.example.org"]),
            "layout " + j(["$HOME/com/", "github/org/repo"]),
            "owner " + names["owner"] + " and " + j([names["login"], "@lab-box"])
            + ", by " + names["public"],
            "root " + j(["root", "@realhost"]) + " vs root@<t6>",
            "runner " + names["runner"],
        ]) + "\n",
        names["runner"] + ".log": "port " + j(["3593", "5"]) + " in a file named after a lab host\n",
        "third_party/x.txt": "excluded " + priv4 + "\n",
        "docs/release/redaction-audit.md": "excluded " + priv4 + "\n",
        "bin.dat": "binary\0" + priv4 + "\n",
    }
    allow = "\n".join([
        "# fixture allow file",
        "drop | * | * | <[A-Za-z0-9_.-]+> | placeholders",
        "drop | * | * | /home/<[^>]*> | placeholder home",
        "drop | som-serial | * | verdin-(am62p|imx8mp)-00000000 | zeroed serial",
        "drop | password | * | NOPASSWD: | sudoers rule",
        "drop | email | * | [A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.invalid | .invalid",
        "info | email | src/* | @upstream\\.example\\.org | upstream author",
        "",
    ])
    private_text = "\n".join([
        "# fixture private values",
        "owner-login: " + names["owner"],
        "public-name: " + names["public"],
        "login: " + names["login"],
        "lab-host: " + names["runner"],
        "",
    ])
    failures = []

    def check(desc, ok):
        print("%s  %s" % ("ok  " if ok else "FAIL", desc))
        if not ok:
            failures.append(desc)

    tmp = tempfile.mkdtemp(prefix="redaction-scan-")
    try:
        repo = os.path.join(tmp, "repo")
        os.makedirs(repo)
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", HOME=tmp,
                   GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid",
                   GIT_COMMITTER_NAME="t",
                   GIT_COMMITTER_EMAIL="t@example.invalid")
        for k in (ENV_VALUES, ENV_FILE):
            env.pop(k, None)
        for path, text in fixtures.items():
            full = os.path.join(repo, path)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w", encoding="utf-8", newline="\n") as f:
                f.write(text)
        for args in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "f"]):
            subprocess.run(["git", "-C", repo, *args], env=env, check=True,
                           capture_output=True)
        allow_path = os.path.join(tmp, "allow")
        with open(allow_path, "w", encoding="utf-8") as f:
            f.write(allow)
        priv_path = os.path.join(tmp, "private")
        with open(priv_path, "w", encoding="utf-8") as f:
            f.write(private_text)

        def scan_with(extra_env, *args):
            e = dict(env, **extra_env)
            p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__),
                                "--allow", allow_path, *args],
                               cwd=repo, env=e, capture_output=True, text=True)
            return p.returncode, p.stdout, p.stderr

        def lines_of(out, key):
            """The hit lines under class KEY's header."""
            got, inside = [], False
            for line in out.splitlines():
                if line.startswith("["):
                    inside = line.split()[1] == key
                elif line.startswith("      ") and inside:
                    got.append(line.strip())
            return got

        def header(out, key):
            return next((ln for ln in out.splitlines()
                         if ln.startswith("[") and ln.split()[1] == key), "")

        rc, out, err = scan_with({})
        check("no private values: exit 1 (findings) %s" % rc, rc == EXIT_FINDING)
        for key in ("owner-login", "lab-host"):
            check("no private values: %s not run" % key,
                  "not run (no private values)" in header(out, key))
        check("no private values: login-at-host checks root only",
              "root only" in header(out, "login-at-host")
              and lines_of(out, "login-at-host") == ["src/a.txt:19  r***[4]@r***[8]"])
        check("no private values: the summary counts 3 classes not run",
              "3 class(es) not run" in out)
        expect = {
            "home-path": ["src/a.txt:6  /***[13]", "src/a.txt:6  /***[12]"],
            "private-ipv4": ["src/a.txt:3  1***[8]", "src/a.txt:4  1***[10]"],
            "ipv6-local": ["src/a.txt:5  f***[7]", "src/a.txt:5  f***[17]"],
            "som-serial": ["src/a.txt:8  v***[21]"],
            "bridge-port": [names["runner"] + ".log:1  3***[5]",
                            "src/a.txt:9  3***[5]"],
            "tailnet-dns": ["src/a.txt:10  b***[19]"],
            "token": ["src/a.txt:11  g***[40]", "src/a.txt:12  -***[26]"],
            "password": ["src/a.txt:13  h***[8]"],
            "email": ["src/a.txt:15  s***[7]@c***[16]",
                      "info src/a.txt:16  d***[3]@u***[20]  (upstream author)"],
            "lab-layout": ["src/a.txt:17  $***[20]"],
        }
        for key, want in expect.items():
            got = lines_of(out, key)
            check("%s: hits %s" % (key, want), got == want)
        check("drop rules: 3 look-alikes dropped (zeroed serial, NOPASSWD, "
              ".invalid)", "3 look-alike(s) dropped:" in out)
        check("RFC 5737 and section numbers are not private IPv4",
              len(lines_of(out, "private-ipv4")) == 2)
        check("third_party/, the audit and binary files are excluded",
              not any(("third_party/" in ln or "redaction-audit.md:" in ln
                       or "bin.dat" in ln) for ln in out.splitlines()
                      if ln.startswith("      ")))
        clear = [priv4, tail4, "someone", "hunter2x", "corp.example", "1a2b3c4d"]
        check("nothing is printed in clear",
              not any(v in out + err for v in clear))

        rc, out, err = scan_with({ENV_FILE: priv_path})
        check("private file: exit 1 (findings) %s" % rc, rc == EXIT_FINDING)
        check("private file: no class not run", "0 class(es) not run" in out)
        check("owner-login hits, masked",
              lines_of(out, "owner-login") == ["src/a.txt:18  o***[11]"])
        check("login-at-host hits root and the private login",
              lines_of(out, "login-at-host") == [
                  "src/a.txt:18  l***[8]@l***[7]", "src/a.txt:19  r***[4]@r***[8]"])
        check("lab-host hits as a whole word, and its file name is masked",
              lines_of(out, "lab-host") == ["src/a.txt:20  r***[28]"]
              and lines_of(out, "bridge-port")[0] == "r***[28].log:1  3***[5]")
        check("private values never printed",
              not any(v in out + err for v in names.values()))
        check("the header counts entries, never values",
              "owner-login x1, public-name x1, login x1, lab-host x1" in out)
        check("owner-login inside the public name is a look-alike",
              "1 look-alike(s) dropped" in header(out, "owner-login"))
        rc2, out2, _ = scan_with({ENV_VALUES: private_text.replace("\n", ";")})
        body = [ln for ln in out.splitlines() if ln.startswith(("[", " "))]
        check("the variable reads the same values", rc2 == rc and body == [
            ln for ln in out2.splitlines() if ln.startswith(("[", " "))])

        inside = os.path.join(repo, "priv.txt")
        with open(inside, "w", encoding="utf-8") as f:
            f.write(private_text)
        rc, _, err = scan_with({ENV_FILE: inside})
        check("a private file inside the repository is refused (exit 2)",
              rc == EXIT_ERROR and "outside" in err)
        rc, _, err = scan_with({ENV_VALUES: "owner: " + names["owner"]})
        check("a bad private entry is refused without echoing it",
              rc == EXIT_ERROR and names["owner"] not in err)
        rc, _, _ = scan_with({}, "no-such-rev")
        check("an unknown revision exits 2", rc == EXIT_ERROR)

        # A clean tree: exit 3 without private values, 0 with them.
        clean = os.path.join(tmp, "clean")
        os.makedirs(clean)
        with open(os.path.join(clean, "ok.txt"), "w", encoding="utf-8") as f:
            f.write("nothing private: 192.0.2.10, root@<board>\n")
        for args in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "c"]):
            subprocess.run(["git", "-C", clean, *args], env=env, check=True,
                           capture_output=True)
        e = dict(env)
        p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__),
                            "--allow", allow_path, "HEAD"], cwd=clean, env=e,
                           capture_output=True, text=True)
        check("clean tree, no private values: exit 3 %s" % p.returncode,
              p.returncode == EXIT_NOT_RUN)
        e[ENV_FILE] = priv_path
        p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__),
                            "--allow", allow_path, "HEAD"], cwd=clean, env=e,
                           capture_output=True, text=True)
        check("clean tree, private values: exit 0 %s" % p.returncode,
              p.returncode == EXIT_OK and "summary: 0 H/M" in p.stdout)

        try:
            load_allow(REPO_ALLOW)
            check("tools/release/redaction-scan.allow parses", True)
        except ScanError as ex:
            check("tools/release/redaction-scan.allow parses: %s" % ex, False)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("%d check(s) failed" % len(failures) if failures else "self-test passed")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser(
        description=__doc__.split("\n\n", 1)[0],
        epilog=__doc__.split("\n\n", 1)[1],
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("rev", nargs="?", default="HEAD",
                    help="revision whose tree is scanned (default HEAD)")
    ap.add_argument("--allow", default=REPO_ALLOW,
                    help="allow file (default: redaction-scan.allow next to "
                         "this script)")
    ap.add_argument("--self-test", action="store_true",
                    help="scan a throwaway fixture repository and check the "
                         "classes, masking, allow rules and exit codes")
    try:
        args = ap.parse_args()
    except SystemExit as e:
        return EXIT_ERROR if e.code else EXIT_OK
    if args.self_test:
        return self_test()
    return run(args.rev, args.allow)


if __name__ == "__main__":
    sys.exit(main())
