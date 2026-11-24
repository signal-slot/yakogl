#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check that STM32MP257F-DK P9 record directories are complete and redacted.

  mp2-p9-record-check.py RECORD_DIR [RECORD_DIR ...]

A RECORD_DIR is records/stm32mp257f-dk/<YYYYMMDD-HHMM>-<name>-<sha8>/
(gitignored), written by tools/board-stm32mp2.sh (RD="$LOGDIR/$(date
+%Y%m%d-%H%M)-$NAME-$sha8"; meta.txt with key=value lines; console.txt).
The writer's sha8 is the first 8 hex digits of the staged image's sha256
(meta image_sha256=); a record that carries commit=, origin_main= or build=
is matched against that too.

Checks, one output line per problem (a redaction line names file, line and
pattern, never the matched text):
  name        <YYYYMMDD-HHMM>-<name>-<sha8>, sha8 8 lowercase hex digits
  commit      a recorded commit or image_sha256 starts with the name's sha8
  files       meta.txt and console.txt exist and are not empty
  tally       a gl_test record (name has gl_test or gltest): console.txt holds
              a `gl_test: all: N of M cases failed` line, or meta.txt has
              no_tally=<reason> (a non-empty reason)
  results     a dEQP record (name has deqp): a non-empty results*.tsv /
              results*.txt, or console.txt naming dEQP-GLES2 cases
  restore     meta.txt records both came back byte for byte:
                u-boot-env: u_boot_env=<back|restored|unchanged|byte-for-byte>
                /boot:      stock_files=unchanged and bootfs_list=staged
                            (the writer's own keys), or boot_restored=<same>
  redaction   no text file (or file name) holds an IPv4 address, a tty device path,
              a home-directory path, or a user@host

Last line: `mp2-p9-record-check: N record(s), M problem(s)`.
Exit 0 only when M is 0; 1 on a problem; 2 for a bad argument.
"""
import os
import re
import sys

NAME_RE = re.compile(r"^(\d{8}-\d{4})-([A-Za-z0-9][A-Za-z0-9._-]*)-([0-9a-f]{8})$")
OK_WORDS = {"back", "restored", "unchanged", "byte-for-byte", "yes", "equal"}
REDACT = [
    ("ipv4", re.compile(r"(?<![0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}(?![0-9])")),
    ("tty-path", re.compile(r"/dev/t" r"ty")),
    ("home-path", re.compile(r"/ho" r"me/")),
    ("login", re.compile(r"[A-Za-z0-9._-]+@[A-Za-z0-9-]+")),
]
TALLY = re.compile(r"^gl_test: all: [0-9]+ of [0-9]+ cases failed", re.M)


def read_text(path):
    with open(path, "rb") as f:
        data = f.read()
    return None if b"\0" in data else data.decode("utf-8", "replace")


def meta_of(text):
    d = {}
    for line in (text or "").splitlines():
        k, sep, v = line.partition("=")
        if sep:
            d[k.strip()] = v.strip()  # the last one wins, as board-stm32mp2.sh mget
    return d


def check(rd):
    probs = []
    base = os.path.basename(os.path.normpath(rd))
    m = NAME_RE.match(base)
    sha8 = m.group(3) if m else None
    if not m:
        probs.append("name: '%s' is not <YYYYMMDD-HHMM>-<name>-<sha8>" % base)
    name = base.lower()

    texts, rels = {}, []
    for d, _, names in os.walk(rd):
        for n in sorted(names):
            p = os.path.join(d, n)
            rel = os.path.relpath(p, rd)
            rels.append(rel)
            t = read_text(p)
            if t is not None:
                texts[rel] = t

    def nonempty(rel):
        return rel in texts and texts[rel].strip() != ""

    for need in ("meta.txt", "console.txt"):
        if need not in rels:
            probs.append("files: %s is missing" % need)
        elif os.path.getsize(os.path.join(rd, need)) == 0 or (need in texts and not texts[need].strip()):
            probs.append("files: %s is empty" % need)
    meta = meta_of(texts.get("meta.txt"))
    con = texts.get("console.txt", "")

    # commit
    cands = [meta.get(k, "") for k in ("commit", "origin_main", "build", "image_sha256")]
    hexc = [c for c in cands if re.fullmatch(r"[0-9a-f]{8,64}", c)]
    if nonempty("meta.txt"):
        if not hexc:
            probs.append("commit: meta.txt records no build commit (commit=, origin_main=, build= or image_sha256=)")
        elif sha8 and not any(c.startswith(sha8) for c in hexc):
            probs.append("commit: the recorded build commit does not start with the name's sha8")

    # tally / results
    if re.search(r"gl_?test", name) and nonempty("console.txt"):
        if not TALLY.search(con) and not meta.get("no_tally"):
            probs.append("tally: console.txt has no 'gl_test: all:' line and meta.txt has no no_tally=<reason>")
    if "deqp" in name:
        res = [r for r in rels if re.fullmatch(r"(.*/)?results[^/]*\.(tsv|txt)", r) and nonempty(r)]
        if not res and "dEQP-GLES2" not in con:
            probs.append("results: no non-empty results*.tsv/results*.txt and no dEQP-GLES2 case in console.txt")

    # restore
    if nonempty("meta.txt"):
        if meta.get("u_boot_env", "").lower() not in OK_WORDS:
            probs.append("restore: meta.txt does not record u-boot-env back byte for byte (u_boot_env=)")
        if meta.get("boot_restored", "").lower() not in OK_WORDS and not (
                meta.get("stock_files") == "unchanged" and meta.get("bootfs_list") == "staged"):
            probs.append("restore: meta.txt does not record /boot back byte for byte (stock_files=unchanged and bootfs_list=staged, or boot_restored=)")

    # redaction
    for rel in sorted(texts):
        for ln, line in enumerate(texts[rel].splitlines(), 1):
            for pn, rx in REDACT:
                if rx.search(line):
                    probs.append("redaction: %s:%d: %s" % (rel, ln, pn))
    for rel in rels:
        for pn, rx in REDACT:
            if rx.search(rel):
                probs.append("redaction: %s: %s in the file name" % (rel, pn))
    return probs


def main(argv):
    args = argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print("usage: mp2-p9-record-check.py RECORD_DIR [RECORD_DIR ...]", file=sys.stderr)
        return 2
    for rd in args:
        if not os.path.isdir(rd):
            print("mp2-p9-record-check: not a directory: " + rd, file=sys.stderr)
            return 2
    total = 0
    for rd in args:
        for p in check(rd):
            print("%s: %s" % (os.path.basename(os.path.normpath(rd)), p))
            total += 1
    print("mp2-p9-record-check: %d record(s), %d problem(s)" % (len(args), total))
    return 0 if total == 0 else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
