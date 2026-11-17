#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_boot_manifest_selftest.py - tools/mp2s_boot_manifest.py, no board.

    python3 -I tools/mp2s_boot_manifest_selftest.py

A fake /boot tree under a temp directory: a record, every way a difference
shows up (content, mode, link target, file <-> symlink, added, removed),
the allowed prefix, the manifest-on-stdin forms, the usage errors, and the
`find . -type f -print0 | sort -z | xargs -0 sha256sum` listing forms the
runner's /boot captures take (relative './' paths and absolute paths; a
listing tracks neither modes nor symlinks).  Python 3 stdlib only.

This is a .py so that the scope map of tools/check.sh keeps it in `tools`:
a new tools/*_selftest.sh maps to 'all' and ran every gate row, gap rows
included, which turned the earlier pull request's gate red.  Its gate row
waits for #341.
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "mp2s_boot_manifest.py")

checks = 0
fails = 0


def ok(name):
    global checks
    checks += 1
    print("ok   %s" % name)


def bad(name, detail):
    global checks, fails
    checks += 1
    fails = 1
    print("FAIL %s: %s" % (name, detail))


def run(args, stdin=None):
    """Run the tool; return (rc, stdout+stderr)."""
    p = subprocess.run([sys.executable, "-I", TOOL] + list(args),
                       input=stdin, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def expect(name, want_rc, pattern, args, stdin=None):
    rc, out = run(args, stdin)
    if rc == want_rc and re.search(pattern, out, re.M):
        ok(name)
    else:
        bad(name, "rc=%d (want %d), no %r in: %s" % (rc, want_rc, pattern, out))


def w(path, text):
    with open(path, "w") as f:
        f.write(text)


def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def snapshot(root):
    """Every entry under root as 'path mode mtime_ns', sorted: proves DIR untouched."""
    out = []
    for dp, dns, fns in os.walk(root):
        for n in fns + dns:
            st = os.lstat(os.path.join(dp, n))
            out.append("%s %o %d" % (os.path.relpath(os.path.join(dp, n), root),
                                     st.st_mode, st.st_mtime_ns))
    return sorted(out)


def listing(root, absolute=False):
    """A `find . -type f -print0 | sort -z | xargs -0 sha256sum` capture of root:
    regular files only, sorted by path, '<sha256><2 spaces><path>'."""
    rels = []
    for dp, dns, fns in os.walk(root):
        for n in fns:
            if not os.path.islink(os.path.join(dp, n)):
                rels.append(os.path.relpath(os.path.join(dp, n), root))
    rels.sort()
    return "".join("%s  %s\n" % (sha(os.path.join(root, r)),
                                 os.path.abspath(os.path.join(root, r)) if absolute else "./" + r)
                   for r in rels)


def main():
    t = tempfile.mkdtemp(prefix="mp2s-boot-manifest.")
    try:
        d = os.path.join(t, "boot")
        os.makedirs(os.path.join(d, "extlinux"))
        os.makedirs(os.path.join(d, "yakogl", "standalone"))
        w(os.path.join(d, "Image"), "kernel\n")
        w(os.path.join(d, "stm32.dtb"), "dtb\n")
        w(os.path.join(d, "extlinux", "extlinux.conf"), "label\n")
        w(os.path.join(d, "yakogl", "standalone", "default.bin"), "old\n")
        os.chmod(os.path.join(d, "Image"), 0o644)
        os.chmod(os.path.join(d, "stm32.dtb"), 0o755)
        os.symlink("Image", os.path.join(d, "Image.lnk"))
        before = snapshot(d)

        rec = os.path.join(t, "rec.txt")
        rec2 = os.path.join(t, "rec2.txt")
        rec3 = os.path.join(t, "rec3.txt")
        rec4 = os.path.join(t, "rec4.txt")

        # ---- record -----------------------------------------------------------
        expect("record", 0, "", ["record", d, "-o", rec])
        rec_text = open(rec).read()
        if re.search(r"^@link\tImage\tImage\.lnk$", rec_text, re.M):
            ok("record has the link")
        else:
            bad("record has the link", rec_text)
        if re.search(r"^@mode\t0755\tstm32\.dtb$", rec_text, re.M):
            ok("record has the mode")
        else:
            bad("record has the mode", rec_text)
        if re.search(r"  Image$", rec_text, re.M):
            ok("record hashes Image")
        else:
            bad("record hashes Image", rec_text)
        rc, out = run(["record", d])
        if rc == 0 and out == rec_text:
            ok("record is stable, stdout equals -o")
        else:
            bad("record is stable, stdout equals -o", "rc=%d: %s" % (rc, out))
        if before == snapshot(d):
            ok("DIR untouched")
        else:
            bad("DIR untouched", "the fake /boot changed while recording")

        # ---- compare: the tree against its own record --------------------------
        expect("same", 0, r"^mp2s-boot: same: 5 files$", ["compare", d, rec])
        rc, out = run(["record", d])
        expect("manifest on stdin", 0, "", ["compare", d, "-"], stdin=out)
        w(os.path.join(d, "Image"), "kernex\n")
        expect("changed byte", 1, r"^changed: Image \(content\)$", ["compare", d, rec])
        expect("changed byte, verdict", 1, r"^mp2s-boot: NOT same: 1 difference", ["compare", d, rec])
        w(os.path.join(d, "Image"), "kernel\n")

        w(os.path.join(d, "new.txt"), "x\n")
        expect("added", 1, r"^added: new\.txt$", ["compare", d, rec])
        os.remove(os.path.join(d, "new.txt"))
        os.remove(os.path.join(d, "stm32.dtb"))
        expect("removed", 1, r"^removed: stm32\.dtb$", ["compare", d, rec])
        w(os.path.join(d, "stm32.dtb"), "dtb\n")
        os.chmod(os.path.join(d, "stm32.dtb"), 0o755)

        os.chmod(os.path.join(d, "Image"), 0o600)
        expect("changed mode", 1, r"mode 0644 -> 0600", ["compare", d, rec])
        os.chmod(os.path.join(d, "Image"), 0o644)

        os.remove(os.path.join(d, "Image.lnk"))
        os.symlink("stm32.dtb", os.path.join(d, "Image.lnk"))
        expect("changed link target", 1, r"link target Image -> stm32\.dtb", ["compare", d, rec])
        os.remove(os.path.join(d, "Image.lnk"))
        w(os.path.join(d, "Image.lnk"), "Image\n")
        expect("link to file", 1, r"link -> file", ["compare", d, rec])
        os.remove(os.path.join(d, "Image.lnk"))
        os.symlink("Image", os.path.join(d, "Image.lnk"))
        expect("restored", 0, r"^mp2s-boot: same: 5 files$", ["compare", d, rec])

        os.remove(os.path.join(d, "stm32.dtb"))
        os.symlink("Image", os.path.join(d, "stm32.dtb"))
        expect("file to link", 1, r"file -> link", ["compare", d, rec])
        os.remove(os.path.join(d, "stm32.dtb"))
        w(os.path.join(d, "stm32.dtb"), "dtb\n")
        os.chmod(os.path.join(d, "stm32.dtb"), 0o755)

        # ---- the allowed prefix: yakogl/standalone/ is ours, everything else is ST's ---
        w(os.path.join(d, "yakogl", "standalone", "default.bin"), "new\n")
        w(os.path.join(d, "yakogl", "standalone", "boot-sha256.txt"), "n\n")
        expect("allowed content", 0,
               r"changed: yakogl/standalone/default\.bin \(content\) \(allowed\)", ["compare", d, rec])
        expect("allowed verdict", 0, r"^mp2s-boot: allowed: ", ["compare", d, rec])
        expect("allow replaces default", 1, r"^mp2s-boot: NOT same: 2 difference",
               ["compare", d, rec, "--allow", "nothing/"])
        expect("allow lists stack", 0, "",
               ["compare", d, rec, "--allow", "yakogl/standalone/", "--allow", "new/"])

        # ---- compare-records ---------------------------------------------------
        expect("record the changed tree", 0, "", ["record", d, "-o", rec3])  # only allowed changes
        expect("records differ, allowed", 0, r"^mp2s-boot: allowed: 2 difference",
               ["compare-records", rec, rec3])
        w(os.path.join(d, "yakogl", "standalone", "default.bin"), "old\n")
        os.remove(os.path.join(d, "yakogl", "standalone", "boot-sha256.txt"))
        with open(os.path.join(d, "Image"), "a") as f:
            f.write("y\n")
        expect("record with the changed byte", 0, "", ["record", d, "-o", rec4])
        expect("records differ", 1, r"^changed: Image \(content\)$", ["compare-records", rec, rec4])
        rc, out = run(["record", d])
        expect("records with stdin", 0, "", ["compare-records", rec4, "-"], stdin=out)
        expect("both stdin", 2, r"only one side", ["compare-records", "-", "-"])
        expect("bad manifest", 2, r"not a manifest", ["compare-records", TOOL, rec])
        expect("missing record file", 2, r"cannot read",
               ["compare-records", os.path.join(t, "nonexistent.txt"), rec])

        # ---- usage and the missing directory -------------------------------------
        expect("no arguments", 2, r"usage", [])
        expect("record: no directory", 2, r"not a directory",
               ["record", os.path.join(t, "nonexistent"), "-o", "-"])
        expect("compare: no directory", 2, r"not a directory",
               ["compare", os.path.join(t, "nonexistent"), rec])
        expect("record: dir and --from", 2, r"exactly one", ["record", d, "--from", rec])

        # ---- the ssh capture forms: sha256sum listings ----------------------------
        # (back to the recorded state first: a capture is then the tree's own record)
        w(os.path.join(d, "Image"), "kernel\n")
        # `cd /boot && find . -type f ... | xargs -0 sha256sum` relative paths, and
        # the interim `find /boot ...` absolute paths: both parse (`./` is stripped,
        # absolute paths are kept as they are), and a listing tracks neither modes
        # nor symlinks.
        cap = listing(d)
        cap_abs = listing(d, absolute=True)
        cap_abs2 = listing(d, absolute=True)
        if cap_abs == cap_abs2:
            ok("absolute capture is stable")
        else:
            bad("absolute capture is stable", "two captures of one tree differ")
        capf = os.path.join(t, "cap.txt")
        cap_abs_f = os.path.join(t, "cap-abs.txt")
        cap_abs2_f = os.path.join(t, "cap-abs2.txt")
        caprec_f = os.path.join(t, "caprec.txt")
        cap2_f = os.path.join(t, "cap2.txt")
        w(capf, cap)
        w(cap_abs_f, cap_abs)
        w(cap_abs2_f, cap_abs2)
        expect("absolute captures compare", 0, r"^mp2s-boot: same: 4 files$",
               ["compare-records", cap_abs_f, cap_abs2_f])
        expect("record --from", 0, "", ["record", "--from", capf, "-o", caprec_f])
        if "@mode" in open(caprec_f).read():
            bad("capture record has no modes", "a capture records modes")
        else:
            ok("capture record has no modes")
        expect("capture equals the tree record", 0, r"^mp2s-boot: same: 4 files$",
               ["compare-records", rec, caprec_f])
        expect("live tree against a capture record", 0, r"^mp2s-boot: same: 5 files$",
               ["compare", d, caprec_f])
        w(os.path.join(d, "Image"), "kernex\n")
        expect("capture into a manifest on stdin", 0, "", ["record", "--from", "-", "-o", cap2_f],
               stdin=listing(d))
        expect("capture sees a changed byte", 1, r"^changed: Image \(content\)$",
               ["compare-records", caprec_f, cap2_f])
        expect("raw listings compare too", 1, r"^changed: Image \(content\)$",
               ["compare-records", capf, cap2_f])
    finally:
        shutil.rmtree(t, ignore_errors=True)

    print("%d checks, %d failed" % (checks, fails))
    return fails


if __name__ == "__main__":
    sys.exit(main())
