# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Sourced by the samples' replay run.sh (tools/slint/replay). Builds libyakogl.a per GPU for
# the Linux OSAL, links a sample's replay program against it and runs it.
#
#   hr_init <repo root>             sets HR_ROOT, HR_HERE, HR_CARGO
#   hr_lib <out> <gpu>              builds <out>/lib-<gpu>/libyakogl.a
#   hr_run <out> <gpu> <program.c> <sample archive> [extra cc args]
#                                   links and runs; the log is <out>/<gpu>.log
#                                   and the exit status is the program's
#   hr_report <out> <gpu>...        prints report.py's summary
#
# The GPUs are the ones src/osal/linux.c can drive: powervr, vivante and v3d
# (the V3D as the host model of the Pi 4's OSAL, src/osal/linux.c "the V3D").
# Each GPU runs with its board's settings from the board layer's
# driver/zephyr/lib/board/<board>/app.conf (hr_board): the V3D's
# CONFIG_YAKOGL_V3D_* (the carve-out).

HR_GPUS_DEFAULT="powervr vivante v3d"

hr_die() { echo "replay: $*" >&2; exit 1; }

hr_init() {
	HR_ROOT="$1"
	HR_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	# capi/yakogl_capi.cmake takes the rustc next to cargo: the first cargo on
	# PATH that has one (a build cache's wrapper may not)
	HR_CARGO=""
	local d
	IFS=: read -r -a path_dirs <<< "$PATH"
	for d in "${path_dirs[@]}"; do
		if [ -x "$d/cargo" ] && [ -x "$d/rustc" ]; then HR_CARGO="$d/cargo"; break; fi
	done
	[ -n "$HR_CARGO" ] || hr_die "no cargo on PATH with a rustc next to it"
}

hr_check_gpus() {
	local g
	for g in "$@"; do
		case "$g" in
		powervr | vivante | v3d) ;;
		*) hr_die "unknown gpu '$g' (powervr, vivante, v3d)" ;;
		esac
	done
}

# The board whose image runs the GPU (its board layer directory,
# driver/zephyr/lib/board/<board>).
hr_board() {
	case "$1" in
	powervr) echo am62p ;;
	vivante) echo imx8mp ;;
	v3d) echo rpi4 ;;
	esac
}

hr_lib() {
	local out="$1" gpu="$2" b="$1/lib-$2"
	[ -f "$b/libyakogl.a" ] && [ -z "${HR_REBUILD:-}" ] && return 0
	echo "replay: libyakogl.a for $gpu"
	cmake -S "$HR_ROOT" -B "$b" -DYAKOGL_OSAL=linux -DYAKOGL_GPU="$gpu" -DYAKOGL_SHADERC=online \
		-DYAKOGL_REPORT_ES2=ON -DYAKOGL_CARGO_PROFILE=release -DYAKOGL_CARGO="$HR_CARGO" > "$b.cmake.log" 2>&1 ||
		{ tail -20 "$b.cmake.log"; hr_die "cmake ($gpu) failed"; }
	cmake --build "$b" --target yakogl_archive -j "$(nproc)" >> "$b.cmake.log" 2>&1 ||
		{ tail -30 "$b.cmake.log"; hr_die "building libyakogl.a ($gpu) failed"; }
}

# The board's settings for the run, as NAME=value lines: each
# CONFIG_YAKOGL_V3D_<NAME> of the board file becomes YAKOGL_V3D_<NAME> for
# src/osal/linux.c (y = 1, n = 0). A setting the environment has already is
# kept.
hr_settings() {
	local conf="$1" name value
	[ -f "$conf" ] || return 0
	while IFS='=' read -r name value; do
		name="${name#CONFIG_}"
		case "$value" in y) value=1 ;; n) value=0 ;; esac
		[ -n "${!name+x}" ] && value="${!name}"
		echo "$name=$value"
	done < <(grep -E '^CONFIG_YAKOGL_V3D_[A-Z0-9_]+=' "$conf")
}

hr_run() {
	local out="$1" gpu="$2" prog="$3" archive="$4"; shift 4
	local bin="$out/replay-$gpu" rc=0 conf
	local -a settings=()
	conf="$HR_ROOT/driver/zephyr/lib/board/$(hr_board "$gpu")/app.conf"
	mapfile -t settings < <(hr_settings "$conf")
	printf '%s\n' "${settings[@]}" > "$out/$gpu-settings.txt"
	cc -O1 -g -Wall -Wextra -I"$HR_ROOT/include" -I"$HR_HERE" -o "$bin" "$prog" "$HR_HERE/replay_common.c" \
		"$@" "$archive" "$out/lib-$gpu/libyakogl.a" -lm -lpthread -ldl ||
		hr_die "linking the $gpu replay failed"
	# every GL error with its entry point, not the first few (src/osal/linux.c)
	env "${settings[@]}" YAKOGL_ERROR_LOG=1000000 "$bin" > "$out/$gpu.log" 2>&1 || rc=$?
	echo "replay: $gpu: exit $rc, log $out/$gpu.log, settings ${settings[*]:-none} ($(hr_board "$gpu")/app.conf)"
	return 0
}

hr_report() {
	local out="$1"; shift
	python3 -I "$HR_HERE/report.py" "$out" "$@" | tee "$out/report.md"
}
