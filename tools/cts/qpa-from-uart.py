#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Rebuild dEQP's .qpa log from a console capture of a deqp image built with
CONFIG_DEQP_ECHO_QPA (driver/samples/zephyr/deqp).

    tools/cts/qpa-from-uart.py uart.log out.qpa
    tools/cts/cts-images.py --out DIR out.qpa

The image prints every log line as "qpa: <line>"; a line longer than the
sample's 511-byte buffer continues on "qpa+ <rest>" lines, which are joined
back without a separator. Every other console line (the sample's
"deqp: ..." lines, driver messages) is dropped. NULs and CRs of a raw serial
stream are removed first.

What it checks, because the serial bridge has lost lines before (RESULTS 1h):
each case must have its "#beginTestCaseResult" and "#endTestCaseResult"
pair, and each PNG image's base64 body must decode. A case that fails either
is reported by name and the exit status is 1; the file is still written, so
the cases that did arrive can be read. An empty rebuild (no "qpa:" line at
all) is exit 2: the image was not built with the option, or the capture is
empty. --self-test checks these verdicts on built-in captures.
"""
import base64
import binascii
import re
import sys

IMG_RE = re.compile(r"<Image\b([^>]*)>(.*?)</Image>", re.S)


def rebuild(text):
    lines = []
    for raw in text.replace("\0", "").replace("\r", "").split("\n"):
        if raw.startswith("qpa: "):
            lines.append(raw[5:])
        elif raw.startswith("qpa+ ") and lines:
            lines[-1] += raw[5:]
    return lines


def check(lines):
    """(case, reason) for every case that is not whole."""
    bad = []
    case = None
    body = []
    for l in lines:
        if l.startswith("#beginTestCaseResult "):
            if case:
                bad.append((case, "no #endTestCaseResult before the next case"))
            case, body = l.split(" ", 1)[1], []
        elif l.startswith(("#endTestCaseResult", "#terminateTestCaseResult")):
            if not case:
                bad.append(("?", "an end with no begin"))
                continue
            for attrs, data in IMG_RE.findall("\n".join(body)):
                if 'Format="PNG"' not in attrs:
                    continue
                try:
                    base64.b64decode("".join(data.split()), validate=True)
                except (ValueError, binascii.Error) as e:
                    bad.append((case, "an image does not decode (%s)" % e))
                    break
            case = None
        elif case:
            body.append(l)
    if case:
        bad.append((case, "no #endTestCaseResult at the end of the capture"))
    return bad


def self_test():
    image = base64.b64encode(b"x" * 9).decode()
    whole = ["#beginTestCaseResult a.b", '<Image Name="R" Format="PNG">' + image + "</Image>", "#endTestCaseResult"]
    text = "boot\nqpa: " + "\nqpa: ".join(whole) + "\ndeqp: Pass a.b\n"
    split = text.replace('Format="PNG">', 'Format="PNG">\nqpa+ ', 1)
    cases = [
        ("a whole case", text, []),
        ("a line the buffer split", split, []),
        ("a lost end", text.replace("qpa: #endTestCaseResult\n", ""), ["a.b"]),
        ("a broken image", text.replace(image, image[:3] + "!" + image[4:]), ["a.b"]),
    ]
    bad = 0
    for desc, t, want in cases:
        got = [c for c, _ in check(rebuild(t))]
        ok = got == want
        print("  %s  %s: %s" % ("ok  " if ok else "FAIL", desc, got or "whole"))
        bad += not ok
    return 1 if bad else 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if len(sys.argv) != 3:
        print("usage: qpa-from-uart.py uart.log out.qpa | --self-test", file=sys.stderr)
        return 2
    with open(sys.argv[1], "rb") as f:
        lines = rebuild(f.read().decode("utf-8", "replace"))
    if not lines:
        print("qpa-from-uart: no 'qpa:' line in %s" % sys.argv[1], file=sys.stderr)
        return 2
    with open(sys.argv[2], "w") as f:
        f.write("\n".join(lines) + "\n")
    bad = check(lines)
    n = sum(1 for l in lines if l.startswith("#beginTestCaseResult "))
    print("qpa-from-uart: %d lines, %d cases, %d not whole" % (len(lines), n, len(bad)))
    for c, why in bad:
        print("  NOT WHOLE %s: %s" % (c, why))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
