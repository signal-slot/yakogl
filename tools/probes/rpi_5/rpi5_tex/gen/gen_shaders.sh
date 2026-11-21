#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_tex: regenerate src/shaders_ours.h, the probe's two programs
# (../shaders/) as our V3D 7.1 online compiler builds them.
#
#   tools/probes/rpi_5/rpi5_tex/gen/gen_shaders.sh           write the header
#   tools/probes/rpi_5/rpi5_tex/gen/gen_shaders.sh --check   compare only
#
# gen_shaders.rs is built as a throwaway crate under target/rpi5_tex_gen/
# (the repository's own gitignored target directory; outside the
# workspace) with a path dependency on yakogl, feature set
# gpu-v3d,shaderc-online,no-shader-table (the 7.1 compiler's, as in
# tests/v3d71_gltest_shaders.rs). Exit 1 when the compiler refuses a program
# (the refusal is printed) or, with --check, when the committed header differs.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
probe=$(dirname "$here")
root=$(cd "$probe/../../../.." && pwd)
work="$root/target/rpi5_tex_gen"
mode=${1:-write}

mkdir -p "$work/src"
cat > "$work/Cargo.toml" <<EOF
[package]
name = "rpi5_tex_gen"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
yakogl = { path = "$root", default-features = false, features = ["host-test", "gpu-v3d", "shaderc-online", "no-shader-table"] }

[workspace]
EOF
cp "$here/gen_shaders.rs" "$work/src/main.rs"

out="$work/shaders_ours.h"
CARGO_TARGET_DIR="$work/target" cargo run --quiet --manifest-path "$work/Cargo.toml" -- \
	"$probe/shaders" "$out"

case "$mode" in
--check)
	if cmp -s "$out" "$probe/src/shaders_ours.h"; then
		echo "rpi5_tex gen_shaders: src/shaders_ours.h is what the compiler builds now"
	else
		diff -u "$probe/src/shaders_ours.h" "$out" || true
		echo "rpi5_tex gen_shaders: src/shaders_ours.h differs from the compiler's output" >&2
		exit 1
	fi
	;;
write)
	cp "$out" "$probe/src/shaders_ours.h"
	echo "rpi5_tex gen_shaders: wrote src/shaders_ours.h"
	;;
*)
	echo "usage: $0 [--check]" >&2
	exit 2
	;;
esac
