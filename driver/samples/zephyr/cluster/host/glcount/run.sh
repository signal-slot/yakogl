#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glcount: the image's own Rust library (../../rust, the same Cargo.toml.in,
# UI, fonts and no_std Slint tree as the image) built for the host and run
# over a stub GL that counts calls (glcount.c says what it answers and what
# it checks). Exit status 0 on PASS, 1 on FAIL.
#
#   AGL_SLINT_CLUSTER_DIR=<clone> SLINT_NOSTD_DIR=<no_std slint tree> \
#   NOSTD_PATCH_DIR=<femtovg/glow forks> host/glcount/run.sh <out-dir>
#
# The variables are the image's (CMakeLists.txt); NOTO_SANS_DIR defaults to
# /usr/share/fonts/noto. GLCOUNT_FRAMES (default 20) sets the frames after
# the check's two. GLCOUNT_DEMO_S=180 replays the demo's sequence instead and
# prints main.c's per-frame means every 10 s (glcount.c's header).
# GLCOUNT_OMIT=<part>[,<part>] draws the UI without those parts, as
# CONFIG_V3D_GL_CLUSTER_OMIT does in the image.
set -eu

out=${1:?usage: run.sh <out-dir>}
: "${AGL_SLINT_CLUSTER_DIR:?the agl-slint-cluster clone}"
: "${SLINT_NOSTD_DIR:?the no_std Slint tree (../../../slint/nostd/README.md)}"
: "${NOSTD_PATCH_DIR:?the no_std femtovg/ and glow/ forks}"
noto=${NOTO_SANS_DIR:-/usr/share/fonts/noto}
here=$(cd "$(dirname "$0")" && pwd)
sample=$(cd "$here/../.." && pwd)
repo=$(cd "$sample/../../../.." && pwd)

mkdir -p "$out/crate"
out=$(cd "$out" && pwd) # absolute: build.rs resolves the UI paths from the crate's directory
sed -e "s#@CARGO_SRC@#$sample/rust#g" -e "s#@SLINT_NOSTD_DIR@#$SLINT_NOSTD_DIR#g" \
	-e "s#@NOSTD_PATCH_DIR@#$NOSTD_PATCH_DIR#g" \
	"$sample/rust/Cargo.toml.in" > "$out/crate/Cargo.toml"

export SLINT_UI_FILE="$sample/ui/zephyr-cluster.slint"
export AGL_SLINT_CLUSTER_UI_DIR="$AGL_SLINT_CLUSTER_DIR/ui"
# GLCOUNT_OMIT=<parts>: the same derived UI as CONFIG_V3D_GL_CLUSTER_OMIT
# (host/ui_omit.py), so a host row and a board row name the same scene
if [ -n "${GLCOUNT_OMIT:-}" ]; then
	python3 "$sample/host/ui_omit.py" "$AGL_SLINT_CLUSTER_DIR/ui" "$out/ui-omit" "$GLCOUNT_OMIT"
	export AGL_SLINT_CLUSTER_UI_DIR="$out/ui-omit"
fi
export SLINT_DEFAULT_FONT="$noto/NotoSans-Regular.ttf"
export SLINT_FONT_PATH="$noto/NotoSans-Light.ttf:$noto/NotoSans-ExtraLight.ttf"
sizes=$(cat "$sample/font-sizes.txt")
[ -n "$sizes" ] || { echo "run.sh: $sample/font-sizes.txt is empty or missing" >&2; exit 1; }
export SLINT_FONT_SIZES="$sizes"
export CARGO_TARGET_DIR="$out/target"
# core and alloc built with the crate's panic = "abort", as the image builds
# them (-Z build-std): the host's prebuilt ones unwind and want
# rust_eh_personality, which a no_std staticlib does not provide
host=$(rustc -vV | sed -n 's/^host: //p')
cargo build --release -Z build-std=core,alloc --target "$host" --manifest-path "$out/crate/Cargo.toml"

# The GL names the stub offers: exactly what YakoGL's eglGetProcAddress
# answers (src/gl/egl.rs, the PROCS table: one "gl!(name)," or "own!(name),"
# per line). A line of another shape stops the run, so the list cannot come
# out short; two known members are checked (the GLES 2.0 core, and the OES
# vertex arrays FemtoVG asks for).
egl="$repo/src/gl/egl.rs"
python3 - "$egl" "$out/gl_names.h" <<'EOF'
import re, sys
src = open(sys.argv[1]).read()
m = re.search(r"^static PROCS: &\[Proc\] = &\[\n(.*?)^\];$", src, re.S | re.M)
if not m:
    sys.exit("run.sh: no PROCS table in " + sys.argv[1])
body = re.sub(r"/\*.*?\*/", "", m.group(1), flags=re.S)
body = re.sub(r"//[^\n]*", "", body)
names = []
for line in body.splitlines():
    if not line.strip():
        continue
    e = re.fullmatch(r"\s*(gl|own)!\(([A-Za-z0-9_]+)\),\s*", line)
    if not e:
        sys.exit("run.sh: a PROCS line of another shape: " + line)
    if e.group(2).startswith("gl"):
        names.append(e.group(2))
open(sys.argv[2], "w").write("".join('"%s",\n' % n for n in names))
EOF
for known in glActiveTexture glGenVertexArraysOES; do
	grep -q "\"$known\"" "$out/gl_names.h" || { echo "run.sh: $known missing from the list" >&2; exit 1; }
done
echo "run.sh: $(wc -l < "$out/gl_names.h") GL names from YakoGL's PROCS"

cc -O1 -Wall -Wextra -Werror -I"$repo/include" -I"$out" -o "$out/glcount" "$here/glcount.c" \
	"$out/target/$host/release/libslint_cluster.a" -lm
"$out/glcount"
