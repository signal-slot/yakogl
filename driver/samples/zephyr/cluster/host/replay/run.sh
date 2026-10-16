#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The cluster's frame sequence through the real library on the host, per GPU
# (README.md). The Rust library is built as glcount/run.sh builds it.
#
#   AGL_SLINT_CLUSTER_DIR=<clone> SLINT_NOSTD_DIR=<no_std slint tree> \
#   NOSTD_PATCH_DIR=<femtovg/glow forks> host/replay/run.sh <out-dir> [gpu...]
#
# gpu: powervr vivante v3d (default: all three). Each GPU runs at its board's
# panel size and, on the V3D, with the Pi 4's CONFIG_YAKOGL_V3D_* (../../boards,
# host-replay/common.sh). REPLAY_FRAMES (default 20) sets the frames after the
# check's two. NOTO_SANS_DIR defaults to /usr/share/fonts/noto.
# Exit status 0 when no GPU found anything.
set -euo pipefail

out=${1:?usage: run.sh <out-dir> [gpu...]}
shift
: "${AGL_SLINT_CLUSTER_DIR:?the agl-slint-cluster clone}"
: "${SLINT_NOSTD_DIR:?the no_std Slint tree (../../../slint/nostd/README.md)}"
: "${NOSTD_PATCH_DIR:?the no_std femtovg/ and glow/ forks}"
noto=${NOTO_SANS_DIR:-/usr/share/fonts/noto}
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
out=$(cd "$out" && pwd) # absolute: build.rs resolves the UI paths from the crate's directory
sed -e "s#@CARGO_SRC@#$sample/rust#g" -e "s#@SLINT_NOSTD_DIR@#$SLINT_NOSTD_DIR#g" \
	-e "s#@NOSTD_PATCH_DIR@#$NOSTD_PATCH_DIR#g" \
	"$sample/rust/Cargo.toml.in" > "$out/crate/Cargo.toml"

export SLINT_UI_FILE="$sample/ui/zephyr-cluster.slint"
export AGL_SLINT_CLUSTER_UI_DIR="$AGL_SLINT_CLUSTER_DIR/ui"
export SLINT_DEFAULT_FONT="$noto/NotoSans-Regular.ttf"
export SLINT_FONT_PATH="$noto/NotoSans-Light.ttf:$noto/NotoSans-ExtraLight.ttf"
sizes=$(cat "$sample/font-sizes.txt")
[ -n "$sizes" ] || hr_die "$sample/font-sizes.txt is empty or missing"
export SLINT_FONT_SIZES="$sizes"
export CARGO_TARGET_DIR="$out/target"
# core and alloc with the crate's panic = "abort", as the image builds them
host=$(rustc -vV | sed -n 's/^host: //p')
cargo build --release -Z build-std=core,alloc --target "$host" --manifest-path "$out/crate/Cargo.toml"
archive="$out/target/$host/release/libslint_cluster.a"

for g in "${gpus[@]}"; do
	hr_lib "$out" "$g"
	hr_run "$out" "$g" "$here/replay.c" "$archive"
done
hr_report "$out" "${gpus[@]}"
