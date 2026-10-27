#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/deps-selftest.sh
#
# Host-only checks of tools/cts/deps-env.sh (deps_env.py) and deps.toml: the
# real deps.toml against an empty scratch layout (names, aliases, board sets,
# markers, packages), and --build / --build-missing against a small fake
# deps.toml (DEPS_TOML) whose downloads are file:// URLs in the scratch
# directory. No network, nothing outside the scratch directory is written.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ENV_SH=$HERE/deps-env.sh
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
# check NAME WANT GOT: GOT must contain WANT
check() { case "$3" in *"$2"*) ok "$1" ;; *) bad "$1: wanted '$2' in: $3" ;; esac; }
absent() { case "$3" in *"$2"*) bad "$1: found '$2' in: $3" ;; *) ok "$1" ;; esac; }

T=$(mktemp -d "${TMPDIR:-/tmp}/deps-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
L=$T/layout
mkdir -p "$L"
export CCFARM_DEPS_DIR=$L
unset CONNECTED_DEPS_DIR   # connected sets it; it would win over the test layout

# --- which directory is the layout -----------------------------------------
H=$T/home
mkdir -p "$H/.local/share/ccfarm/deps/yakogl"
out=$(env -u CCFARM_DEPS_DIR HOME=$H "$ENV_SH" --layout 2>&1)
check "--layout: the old directory on a machine with only it" "$H/.local/share/ccfarm/deps/yakogl" "$out"
mkdir -p "$H/.local/share/connected/deps/yakogl"
out=$(env -u CCFARM_DEPS_DIR HOME=$H "$ENV_SH" --layout 2>&1)
check "--layout: connected's directory once it exists" "$H/.local/share/connected/deps/yakogl" "$out"
out=$(env -u CCFARM_DEPS_DIR CONNECTED_DEPS_DIR=$L HOME=$H "$ENV_SH" --layout 2>&1)
check "--layout: CONNECTED_DEPS_DIR first" "$L" "$out"
out=$("$ENV_SH" --layout 2>&1)
check "--layout: CCFARM_DEPS_DIR (connected sets both names)" "$L" "$out"

# --- the real deps.toml, an empty layout ------------------------------------
out=$("$ENV_SH" --check 2>&1); rc=$?
[ "$rc" -eq 1 ] && ok "--check on an empty layout exits 1" || bad "--check on an empty layout: rc $rc"
check "--check names the layout" "layout   $L" "$out"
check "--check marks a missing entry" "missing  ZEPHYR_WS  $L/zephyr-ws/.west/config" "$out"
check "--check has the MESA_PAN_TREE marker" "missing  MESA_PAN_TREE  $L/mesa-pan/mesa-26.2.1/build-pan/build.ninja" "$out"
check "--check counts the entries" "entries missing in $L" "$out"
check "--check accepts the layout itself as a marker" "ok       GLSL2USC_CACHE  $L" "$out"
absent "--check without a board has no board entry" DEQP_WORK "$out"
check "--check has the Mali firmware entry" "missing  MALI_CSFFW  $L/mali-csffw/mali_csffw.bin" "$out"
check "--check has the rustup target entry" "RUST_TARGET_AARCH64_LINUX  $(rustc --print sysroot)/lib/rustlib/aarch64-unknown-linux-gnu/lib" "$out"

out=$("$ENV_SH" 2>&1)
check "env prints a name" "ZEPHYR_WS=$L/zephyr-ws" "$out"
check "env prints an alias" "ZEPHYR_WORKSPACE=$L/zephyr-ws" "$out"
check "env prints MESA_PAN_TREE" "MESA_PAN_TREE=$L/mesa-pan/mesa-26.2.1" "$out"
[ -d "$L/glsl2usc-cache" ] && ok "env makes GLSL2USC_CACHE" || bad "env did not make GLSL2USC_CACHE"
out=$("$ENV_SH" imx8mp 2>&1)
check "a board adds its tree" "DEQP_WORK=$L/deqp-stage1-imx8mp" "$out"
check "a board adds a board=\"*\" entry" "NOSTD_PATCH_DIR=$L/nostd" "$out"
check "imx8mp adds IMX8MP_OUT" "IMX8MP_OUT=$L/imx8mp-out" "$out"
out=$("$ENV_SH" rpi_4b 2>&1)
absent "rpi_4b has no IMX8MP_OUT" IMX8MP_OUT "$out"
out=$("$ENV_SH" nosuchboard 2>&1); rc=$?
[ "$rc" -eq 2 ] && ok "an unknown board exits 2" || bad "an unknown board: rc $rc"

out=$("$ENV_SH" --packages apt 2>&1)
check "apt packages: MESA_PAN_TREE's" "bison" "$out"
check "apt packages: libclc-21-dev" "libclc-21-dev" "$out"
n=$(printf '%s\n' "$out" | sort | uniq -d | wc -l)
[ "$n" -eq 0 ] && ok "apt packages are listed once" || bad "apt packages repeat"
out=$("$ENV_SH" --packages portage 2>&1)
check "portage packages" "dev-build/meson" "$out"
out=$("$ENV_SH" --packages yum 2>&1); rc=$?
[ "$rc" -eq 2 ] && ok "an unknown package family exits 2" || bad "an unknown family: rc $rc"

# --- --build against a fake deps.toml ---------------------------------------
printf 'payload\n' > "$T/a.tar"
sum=$(sha256sum "$T/a.tar" | cut -c1-64)
printf 'fake: %s\n' "$sum" > "$T/UPSTREAM"
cat > "$T/deps.toml" <<EOF
[[dep]]
name = "A"
aliases = ["A_ALIAS"]
path = "a"
marker = "a/done"
build = '''
echo run >> "\$DEPS/a.runs"
mkdir -p "\$DEPS/a.part"
cp "\$DEPS/downloads/a.tar" "\$DEPS/a.part/done"
mv "\$DEPS/a.part" "\$DEPS/a"
'''
[[dep.inputs]]
url = "file://$T/a.tar"
sha256 = "$sum"

[[dep]]
name = "B"
path = "b"
marker = "b/done"
requires = ["A"]
build = 'mkdir -p "\$DEPS/b" && cat "\$DEPS/a/done" > "\$DEPS/b/done"'

[[dep]]
name = "C"
path = "c"
marker = "c"
note = "not reproducible"

[[dep]]
name = "D"
path = "d"
marker = "d"
build = 'echo ran >> "\$DEPS/d.runs"; exit 1'

[[dep]]
name = "E"
path = "e"
marker = "e/done"
build = 'true'

[[dep]]
name = "F"
path = "f"
marker = "f"
build = 'mkdir -p "\$DEPS/f"'
[[dep.inputs]]
url = "file://$T/a.tar"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[[dep]]
name = "G"
path = "g"
marker = "g"
build = 'mkdir -p "\$DEPS/g"'
[[dep.inputs]]
url = "file://$T/a.tar"
sha256_from = "$T/UPSTREAM:fake"

[[dep]]
name = "W"
path = "w-{board}"
marker = "w-{board}/done"
board = "*"
build = 'mkdir -p "\$DEPS/w-\$BOARD_NAME" && touch "\$DEPS/w-\$BOARD_NAME/done"'
EOF
export DEPS_TOML=$T/deps.toml

out=$("$ENV_SH" --build B 2>&1); rc=$?
[ "$rc" -eq 0 ] && ok "--build B (requires A) succeeds" || bad "--build B: rc $rc: $out"
check "--build makes the required entry first" "build    A" "$out"
[ "$(cat "$L/b/done" 2>/dev/null)" = payload ] && ok "B's build saw A's tree" || bad "B's marker is wrong"
out=$("$ENV_SH" --check 2>&1 | grep -E '^ok +(A|B) ' | wc -l)
[ "$out" -eq 2 ] && ok "--check turns A and B ok" || bad "--check ok lines for A and B: $out"
out=$("$ENV_SH" --build A_ALIAS B 2>&1); rc=$?
[ "$rc" -eq 0 ] && ok "--build of present entries succeeds (alias accepted)" || bad "--build again: rc $rc"
check "--build leaves a present entry alone" "present  A" "$out"
[ "$(wc -l < "$L/a.runs")" -eq 1 ] && ok "A's build ran once" || bad "A's build ran again"

out=$("$ENV_SH" --build C 2>&1); rc=$?
[ "$rc" -eq 1 ] && ok "--build of an entry without a build exits 1" || bad "--build C: rc $rc"
check "--build C prints the note" "not reproducible" "$out"
out=$("$ENV_SH" --build D 2>&1); rc=$?
[ "$rc" -eq 1 ] && ok "a failing build exits 1" || bad "--build D: rc $rc"
check "a failing build is named" "failed   D" "$out"
out=$("$ENV_SH" --build E 2>&1); rc=$?
[ "$rc" -eq 1 ] && check "a build that leaves no marker fails" "build finished but" "$out" || bad "--build E: rc $rc"
out=$("$ENV_SH" --build F 2>&1); rc=$?
[ "$rc" -eq 1 ] && ok "a wrong sha256 exits 1" || bad "--build F: rc $rc"
check "a wrong sha256 is reported" "expected 0000" "$out"
[ ! -e "$L/f" ] && ok "a wrong sha256 builds nothing" || bad "F was built"
out=$("$ENV_SH" --build G 2>&1); rc=$?
[ "$rc" -eq 0 ] && ok "sha256_from reads the digest from a file" || bad "--build G: rc $rc: $out"
out=$("$ENV_SH" --build NOSUCH 2>&1); rc=$?
[ "$rc" -eq 2 ] && ok "--build of an unknown entry exits 2" || bad "--build NOSUCH: rc $rc"
out=$("$ENV_SH" --build W 2>&1); rc=$?
[ "$rc" -eq 2 ] && check "a per-board entry needs a board" "per board" "$out" || bad "--build W: rc $rc"

out=$(BOARD_NAME=am62p "$ENV_SH" --build-missing am62p 2>&1); rc=$?
[ "$rc" -eq 1 ] && ok "--build-missing exits 1 while an entry cannot be made" || bad "--build-missing: rc $rc"
check "--build-missing makes the board's entry" "built    W  $L/w-am62p/done" "$out"
check "--build-missing names what it could not make" "not made: C D E F" "$out"
[ "$(wc -l < "$L/d.runs")" -eq 2 ] && ok "--build-missing retried D (still missing)" || bad "D's runs: $(cat "$L/d.runs")"

# The gate workflow builds MALI_CSFFW on a runner without it: the real
# entry's path, marker and build, on a stand-in download (file://, its own
# digest) named as the pinned URL names the firmware.
printf 'not the firmware\n' > "$T/mali_csffw.bin"
python3 -I - "$HERE/deps.toml" "$T" > "$T/mali.toml" <<'PY' || bad "the MALI_CSFFW entry: no stand-in made"
import hashlib, json, sys, tomllib
d = next(e for e in tomllib.load(open(sys.argv[1], "rb"))["dep"] if e["name"] == "MALI_CSFFW")
if d["inputs"][0]["url"].rsplit("/", 1)[1] != "mali_csffw.bin":
    sys.exit("MALI_CSFFW: the pinned URL does not name mali_csffw.bin, the download its build copies")
t = sys.argv[2]
sha = hashlib.sha256(open(f"{t}/mali_csffw.bin", "rb").read()).hexdigest()
q = json.dumps  # a JSON string is a TOML basic string
print(f'[[dep]]\nname = "M"\npath = {q(d["path"])}\nmarker = {q(d["marker"])}\nbuild = {q(d["build"])}')
print(f'[[dep.inputs]]\nurl = "file://{t}/mali_csffw.bin"\nsha256 = "{sha}"')
PY
out=$(DEPS_TOML=$T/mali.toml "$ENV_SH" --build M 2>&1); rc=$?
[ "$rc" -eq 0 ] && cmp -s "$T/mali_csffw.bin" "$L/mali-csffw/mali_csffw.bin" &&
  ok "MALI_CSFFW's build puts the pinned download at its marker" || bad "MALI_CSFFW's build: rc $rc: $out"
[ ! -e "$L/mali-csffw/mali_csffw.bin.part" ] && ok "MALI_CSFFW's build leaves no .part" || bad "MALI_CSFFW: .part left"

echo
if [ "$fail" -eq 0 ]; then
  echo "== deps-selftest: all checks passed"
else
  echo "== deps-selftest: $fail check(s) FAILED" >&2
fi
[ "$fail" -eq 0 ]
