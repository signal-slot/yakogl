#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_sim_qemu_diff.sh: differential check. For counted defaults of 1, 2 and 3
# the host simulator (tools/mp2s_script_sim.py, run on the product script
# driver/tools/mp2-standalone/mp2-standalone.cmd) and real U-Boot v2023.10 in
# QEMU (driver/tools/mp2-standalone/qemu-product/product.py, imported with its
# case list replaced, not edited; through ST's distro loop) must agree on:
#
#   - per boot, the script's console events in order: "boots left N" (left=N),
#     loaded, go, image (the image ran), "default used up" (usedup),
#     "no default set" (nodefault);
#   - the first boot that reaches the stock path without the image having run;
#   - the final saved boot_prefixes and mp2s_* variables.
#
# The simulator runs without --image-returns (a Zephyr image resets the SoC, so
# it reaches the stock path only after the last count); the QEMU stand-in image
# returns and the stock marker follows it on every boot, which is why "stock" is
# compared as the first image-free boot, not per boot.
#
#   tools/mp2s_sim_qemu_diff.sh                              run both, N=1,2,3
#   tools/mp2s_sim_qemu_diff.sh --compare SIM.txt QEMU.txt   compare two sequences
#   tools/mp2s_sim_qemu_diff.sh --normalize-sim              sim output (stdin) -> sequence
#
# Prints "mp2s_sim_qemu_diff: N=<n> agree" per N and "mp2s_sim_qemu_diff: all
# agree" (exit 0); a disagreement prints the two sequences and exits 1. Without
# QEMU_SYSTEM_AARCH64 and UBOOT_V2023_10_SRC (default $CONNECTED_DEPS_DIR/
# u-boot-v2023.10) it prints "skip: <what>" and exits 0.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/.." && pwd)
std=$root/driver/tools/mp2-standalone
skip() { echo "skip: $*"; exit 0; }

helper() {
	cat <<'PY'
import importlib.util, os, re, sys

LEFT_RE = re.compile(r"boots left (\d+)")


def events_from(text):
    """Tokens of console lines (either side), in order."""
    out = []
    for ln in text:
        if "mp2s: " in ln:
            m = LEFT_RE.search(ln)
            if m:
                out.append("left=" + m.group(1))
            elif "mp2s: loaded " in ln:
                out.append("loaded")
            elif "mp2s: go 0x" in ln:
                out.append("go")
            elif "default used up" in ln:
                out.append("usedup")
            elif "no default set" in ln:
                out.append("nodefault")
        if "mp2q: image ran" in ln or re.match(r"sim: boot \d+: the image runs", ln):
            out.append("image")
    return out


def canon(boots, stock_first, bp, mp2s):
    lines = ["boot %d: %s" % (i, " ".join(b) or "-") for i, b in enumerate(boots, 1)]
    lines.append("first-stock: %s" % stock_first)
    lines.append("env: boot_prefixes=%s mp2s=%s" % (
        bp, ",".join("%s=%s" % kv for kv in sorted(mp2s.items())) or "none"))
    return "\n".join(lines) + "\n"


def sim_canon(text):
    boots, cur, bp, mp2s, stock = [], None, None, {}, None
    for ln in text.splitlines():
        m = re.match(r"sim: boot (\d+): ", ln)
        if m:
            n = int(m.group(1))
            while len(boots) < n:
                boots.append([])
            cur = n - 1
        if cur is not None and ("mp2s: " in ln or "the image runs" in ln):
            boots[cur] += events_from([ln])
        m = re.match(r"sim: result: Linux at boot (\d+)", ln)
        if m:
            stock = int(m.group(1))
        if ln.startswith("sim: env: boot_prefixes"):
            bp = re.search(r'boot_prefixes "([^"]*)"', ln).group(1)
            for k, v in re.findall(r"(mp2s_\w+)=([^,\s]+)", ln):
                mp2s[k] = v
    return canon(boots, stock, bp, mp2s)


def parse(path):
    d = {"boots": {}, "stock": None, "env": None}
    for ln in open(path).read().splitlines():
        m = re.match(r"boot (\d+): (.*)$", ln)
        if m:
            d["boots"][int(m.group(1))] = [] if m.group(2) == "-" else m.group(2).split()
        elif ln.startswith("first-stock: "):
            d["stock"] = ln[13:]
        elif ln.startswith("env: "):
            d["env"] = ln[5:]
    return d


def compare(a, b):
    """The disagreements between two canonical sequences (empty: agree)."""
    A, B = parse(a), parse(b)
    bad = []
    if not A["boots"] or not B["boots"]:
        bad.append("an empty sequence (sim %d boots, qemu %d boots)" % (len(A["boots"]), len(B["boots"])))
    for n in sorted(set(A["boots"]) | set(B["boots"])):
        x, y = A["boots"].get(n), B["boots"].get(n)
        if x != y:
            bad.append("boot %d: sim %s, qemu %s" % (n, x, y))
    if A["stock"] != B["stock"]:
        bad.append("first stock boot: sim %s, qemu %s" % (A["stock"], B["stock"]))
    if A["env"] != B["env"]:
        bad.append("final env: sim %s, qemu %s" % (A["env"], B["env"]))
    return bad


def qemu_run(a):
    """Imports qemu-product/product.py and replaces its cases() with one case per
    N (default N, then N+1 untouched boots) whose check writes the sequence."""
    pdir, out, n_list = a[0], a[1], [int(x) for x in a[2].split(",")]
    sys.path.insert(0, pdir)
    spec = importlib.util.spec_from_file_location("mp2s_product", os.path.join(pdir, "product.py"))
    prod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(prod)

    def mk(n):
        def check(setup, boots):
            seq = [events_from(b.lines) for b in boots]
            stock = next((b.n for b in boots if prod.STOCK in b.lines and prod.IMAGE not in b.lines), None)
            last = boots[-1].saved
            mp2s = {k: v for k, v in last.items() if k.startswith("mp2s_")}
            with open(os.path.join(out, "qemu-%d.txt" % n), "w") as f:
                f.write(canon(seq, stock, last.get("boot_prefixes"), mp2s))
            return []
        return prod.Case("diff_%d" % n, "differential check, default %d" % n,
                         prod.default_presets(n), ["full"] * (n + 1), check)
    prod.cases = lambda: [mk(n) for n in n_list]
    sys.argv = ["product.py"] + a[3:]
    return prod.main()


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "sim":
        sys.stdout.write(sim_canon(sys.stdin.read()))
    elif mode == "compare":
        bad = compare(sys.argv[2], sys.argv[3])
        for b in bad:
            print("DISAGREE: " + b)
        sys.exit(1 if bad else 0)
    elif mode == "qemu":
        sys.exit(qemu_run(sys.argv[2:]))
PY
}

tmp=${TMPDIR:-$root/target}
mkdir -p "$tmp"
work=$(mktemp -d "$tmp/sqdiff.XXXXXX")
trap 'rm -rf "$work"' EXIT
helper > "$work/diff.py"

case "${1:-}" in
--compare)
	[ $# -eq 3 ] || { echo "usage: $0 --compare SIM.txt QEMU.txt" >&2; exit 2; }
	python3 -I "$work/diff.py" compare "$2" "$3"
	exit ;;
--normalize-sim)
	python3 -I "$work/diff.py" sim
	exit ;;
esac

qemu=${QEMU_SYSTEM_AARCH64:-}
[ -n "$qemu" ] && [ -x "$qemu" ] || skip "QEMU_SYSTEM_AARCH64 (unset or not executable)"
if [ -z "${UBOOT_V2023_10_SRC:-}" ]; then
	if [ -n "${CONNECTED_DEPS_DIR:-}" ] && [ -f "$CONNECTED_DEPS_DIR/u-boot-v2023.10/Makefile" ]; then
		export UBOOT_V2023_10_SRC=$CONNECTED_DEPS_DIR/u-boot-v2023.10
	else
		skip "UBOOT_V2023_10_SRC (unset, and no \$CONNECTED_DEPS_DIR/u-boot-v2023.10)"
	fi
fi
[ -f "$UBOOT_V2023_10_SRC/Makefile" ] || skip "UBOOT_V2023_10_SRC ($UBOOT_V2023_10_SRC has no Makefile)"
# shellcheck source=../driver/tools/mp2-standalone/qemu/build-uboot.sh
. "$std/qemu/build-uboot.sh"
cross=$(mp2_qemu_cross)
[ -n "$cross" ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
for t in python3 mke2fs sfdisk truncate; do
	command -v "$t" > /dev/null || [ -x "/usr/sbin/$t" ] || [ -x "/sbin/$t" ] || skip "$t (not found)"
done

rc=0
uboot=$(bash "$std/qemu/build-uboot.sh") || rc=$?
[ "$rc" -ne 77 ] || skip "CROSS_COMPILE (no aarch64 cross compiler)"
[ "$rc" -eq 0 ] || { echo "mp2s_sim_qemu_diff: build-uboot.sh failed ($rc)"; exit 1; }
uboot=$(echo "$uboot" | tail -n 1)
mkdir -p "$work/script" "$work/logs"
bash "$std/build.sh" "$work/script" > /dev/null
"${cross}gcc" -nostdlib -c -DIMGSZ=0x2000 -o "$work/standin.o" "$std/qemu/image.S"
"${cross}objcopy" -O binary -j .text "$work/standin.o" "$work/standin.bin"
truncate -s 4096 "$work/standin.bin"

rc=0
python3 -I "$work/diff.py" qemu "$std/qemu-product" "$work" 1,2,3 \
	--qemu "$qemu" --uboot "$uboot" --image "$work/standin.bin" \
	--script "$work/script/boot.scr.uimg" \
	--fixture "$std/st-env/stm32mp25-dk-initial-env.txt" \
	--work "$work/work" --logs "$work/logs" \
	--uboot-script "$root/tools/uboot_script.py" > "$work/qemu.log" 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
	tail -n 30 "$work/qemu.log"
	echo "mp2s_sim_qemu_diff: the QEMU run failed ($rc)"
	exit 1
fi

fail=0
for n in 1 2 3; do
	python3 -I "$root/tools/mp2s_script_sim.py" run "$std/mp2-standalone.cmd" --left "$n" \
		| python3 -I "$work/diff.py" sim > "$work/sim-$n.txt"
	if [ ! -f "$work/qemu-$n.txt" ]; then
		echo "DISAGREE: N=$n: no QEMU sequence (not every boot ran)"; fail=1; continue
	fi
	if python3 -I "$work/diff.py" compare "$work/sim-$n.txt" "$work/qemu-$n.txt" > "$work/d.txt"; then
		echo "mp2s_sim_qemu_diff: N=$n agree"
	else
		echo "mp2s_sim_qemu_diff: N=$n DISAGREE"; cat "$work/d.txt"
		echo "--- sim"; cat "$work/sim-$n.txt"; echo "--- qemu"; cat "$work/qemu-$n.txt"
		fail=1
	fi
done
[ "$fail" -eq 0 ] || exit 1
echo "mp2s_sim_qemu_diff: all agree"
