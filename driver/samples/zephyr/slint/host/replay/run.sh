#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The Slint sample's frame sequence through the real library on the host, per
# GPU (README.md). The Rust archive is built as slint/CMakeLists.txt builds it
# (for the host instead of aarch64-unknown-none).
#
#   SLINT_NOSTD_DIR=<no_std slint tree> NOSTD_PATCH_DIR=<femtovg/glow forks> \
#   host/replay/run.sh <out-dir> [gpu...]
#
# gpu: powervr vivante v3d (default: all three). REPLAY_FRAMES (default 300)
# sets the animation frames after the first one. The v3d run takes the Pi 4's
# CONFIG_YAKOGL_V3D_* from ../../boards/rpi_4b.conf (host-replay/common.sh).
# The UI's font is "Noto Sans": a host without it installed passes
# NOTO_SANS_DIR (NotoSans-Light/-Regular/-SemiBold.ttf), which goes to the
# Slint compiler as SLINT_FONT_PATH.
# Exit status 0 when no GPU found anything.
set -euo pipefail

out=${1:?usage: run.sh <out-dir> [gpu...]}
shift
: "${SLINT_NOSTD_DIR:?the no_std Slint tree (../../nostd/README.md)}"
: "${NOSTD_PATCH_DIR:?the no_std femtovg/ and glow/ forks}"
here=$(cd "$(dirname "$0")" && pwd)
sample=$(cd "$here/../.." && pwd)
repo=$(cd "$sample/../../../.." && pwd)

# shellcheck source=../../../host-replay/common.sh
. "$sample/../host-replay/common.sh"
hr_init "$repo"
gpus=("$@")
[ ${#gpus[@]} -gt 0 ] || read -r -a gpus <<< "$HR_GPUS_DEFAULT"
hr_check_gpus "${gpus[@]}"

mkdir -p "$out/crate"
out=$(cd "$out" && pwd)
sed -e "s#@CARGO_SRC@#$sample/rust#g" -e "s#@SLINT_CHECK_DIR@#$sample/check#g" \
	-e "s#@SLINT_NOSTD_DIR@#$SLINT_NOSTD_DIR#g" -e "s#@NOSTD_PATCH_DIR@#$NOSTD_PATCH_DIR#g" \
	"$sample/rust/Cargo.toml.in" > "$out/crate/Cargo.toml"
left=$(grep -v '^#' "$out/crate/Cargo.toml" | grep -o '@[A-Z_]*@' | sort -u | tr '\n' ' ' || true)
[ -z "$left" ] || hr_die "Cargo.toml.in has a placeholder run.sh does not fill: $left"

export SLINT_UI_FILE="$sample/ui/cluster.slint"
export SAFEUI_LAYOUT_DIR="$sample/../safeui/ui/1024x600" # safeui_dir.cmake's default
if [ -n "${NOTO_SANS_DIR:-}" ]; then
	export SLINT_FONT_PATH="$NOTO_SANS_DIR/NotoSans-Light.ttf:$NOTO_SANS_DIR/NotoSans-Regular.ttf:$NOTO_SANS_DIR/NotoSans-SemiBold.ttf"
fi
export CARGO_TARGET_DIR="$out/target"
# core and alloc with the crate's panic = "abort", as the image builds them
host=$(rustc -vV | sed -n 's/^host: //p')
cargo build --release -Z build-std=core,alloc --target "$host" --manifest-path "$out/crate/Cargo.toml"
archive="$out/target/$host/release/libslint_sample.a"

for g in "${gpus[@]}"; do
	hr_lib "$out" "$g"
	hr_run "$out" "$g" "$here/replay.c" "$archive"
done
hr_report "$out" "${gpus[@]}"
