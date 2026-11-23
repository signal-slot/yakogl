# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# t6-cpufreq - the CPU clock of the CPU a NanoPC-T6 jump runs on (issue
# #11468): its cpufreq policy's facts, a hold of that policy at one step,
# and the hold's undo. POSIX sh, no shebang: nanopc-t6.sh sends this text to
# the board ahead of the command that calls it (as who_remote.sh), and
# board_lock_selftest.sh runs it on a made-up sysfs. The board's /sys and
# /proc are read under T6CF_ROOT (empty on the board, the made-up root in the
# self-test).
#
# Why: the image does not set the CPU clock; it runs at the frequency
# Linux's governor left on the CPU at the jump (ondemand on the T6, 408 MHz
# to 1.8 GHz).
#
# Which CPU the jump runs on (linux v6.18):
#  - the normal route, reboot(LINUX_REBOOT_CMD_KEXEC): kernel_kexec runs
#    migrate_to_reboot_cpu() (kernel/kexec_core.c:1186-1187), which pins the
#    task to reboot_cpu, or to the first online CPU when that one is offline
#    (kernel/reboot.c:250-266), and machine_kexec (kexec_core.c:1202) enters
#    the image on it. reboot_cpu reads at /sys/kernel/reboot/cpu
#    (kernel/reboot.c:1326-1352). When that file cannot be read, the CPU is
#    not known and every policy is printed.
#  - the crash route: the image starts on the CPU that panics, and
#    kexec_zephyr triggers the panic only when CPU 0 is the one online CPU
#    (kexec_zephyr.c, "The crash route").
#
# What the hold changes: scaling_min_freq and scaling_max_freq of that
# policy, both set to the step, written in the order that keeps min <= max
# after each write; nothing else (no governor change, no module). Every
# frequency target the kernel then sets is clamped to [min, max]
# (drivers/cpufreq/cpufreq.c:2358-2359, :502), also the one device_shutdown
# sets on the normal route (drivers/base/core.c:4796, cpufreq_suspend, which
# stops the governor and sets the policy's suspend_freq when the driver has
# one, cpufreq.c:1939-1958). Without a hold, that is a change of the clock
# between the record and the jump on the normal route.

# cf_read <file>: its value on one line, or "unreadable".
cf_read() {
	if cf_v=$(cat "$1" 2>/dev/null); then
		printf '%s\n' "$cf_v" | tr -s ' \n' '  ' | sed 's/^ //; s/ $//'
	else
		echo unreadable
	fi
}

# cf_in_list <n> <list>: n is in a sysfs CPU list ("0-3,5").
cf_in_list() {
	for cf_r in $(printf '%s\n' "$2" | tr ',' ' '); do
		case $cf_r in
		*-*) [ "$1" -ge "${cf_r%-*}" ] && [ "$1" -le "${cf_r#*-}" ] && return 0 ;;
		*) [ "$1" = "$cf_r" ] && return 0 ;;
		esac
	done
	return 1
}

# cf_jump_cpu <normal|crash>: sets cf_cpu (empty when unknown) and cf_why.
cf_jump_cpu() {
	cf_cpu=
	if [ "$1" = crash ]; then
		cf_cpu=0
		cf_why="crash route: kexec_zephyr panics only with CPU 0 the one online CPU"
		return 0
	fi
	cf_rcpu=$(cat "${T6CF_ROOT:-}/sys/kernel/reboot/cpu" 2>/dev/null)
	cf_online=$(cat "${T6CF_ROOT:-}/sys/devices/system/cpu/online" 2>/dev/null)
	case $cf_rcpu in
	'' | *[!0-9]*)
		cf_why="normal route: /sys/kernel/reboot/cpu unreadable ('$cf_rcpu')"
		return 0
		;;
	esac
	case $cf_online in
	'' | *[!0-9,-]*)
		cf_why="normal route: reboot_cpu $cf_rcpu, /sys/devices/system/cpu/online unreadable ('$cf_online')"
		return 0
		;;
	esac
	if cf_in_list "$cf_rcpu" "$cf_online"; then
		cf_cpu=$cf_rcpu
		cf_why="normal route: reboot_cpu $cf_rcpu (/sys/kernel/reboot/cpu), online ($cf_online)"
	else
		cf_cpu=${cf_online%%[-,]*}
		cf_why="normal route: reboot_cpu $cf_rcpu is offline, so the first online CPU ($cf_online)"
	fi
}

# cf_policy_line <policy directory>: its facts on one line.
cf_policy_line() {
	printf '%s (CPUs %s): driver %s, governor %s, scaling_cur_freq %s, scaling_min_freq %s, scaling_max_freq %s, scaling_available_frequencies %s, cpuinfo_cur_freq %s\n' \
		"${1##*/}" "$(cf_read "$1/related_cpus")" "$(cf_read "$1/scaling_driver")" \
		"$(cf_read "$1/scaling_governor")" "$(cf_read "$1/scaling_cur_freq")" \
		"$(cf_read "$1/scaling_min_freq")" "$(cf_read "$1/scaling_max_freq")" \
		"$(cf_read "$1/scaling_available_frequencies")" "$(cf_read "$1/cpuinfo_cur_freq")"
}

# cf_facts <normal|crash>: the jump CPU and its policy, as name=value lines
# (cpufreq_cpu, empty when unknown, cpufreq_why, cpufreq_policy, "absent"
# when the CPU has none, cpufreq_cur, _min, _max, _steps, and cpufreq_line,
# the policy's facts on one line, or every policy's when the CPU is unknown).
# Sets cf_dir to the policy's directory (empty when there is none).
cf_facts() {
	cf_jump_cpu "$1"
	cf_dir=
	printf 'cpufreq_cpu=%s\ncpufreq_why=%s\n' "$cf_cpu" "$cf_why"
	if [ -z "$cf_cpu" ]; then
		echo cpufreq_policy=absent
		cf_all=
		for cf_p in "${T6CF_ROOT:-}"/sys/devices/system/cpu/cpufreq/policy*; do
			[ -d "$cf_p" ] || continue
			cf_all="$cf_all${cf_all:+; }$(cf_policy_line "$cf_p")"
		done
		printf 'cpufreq_line=every policy: %s\n' "${cf_all:-none}"
		return 0
	fi
	if ! cf_d=$(cd -P "${T6CF_ROOT:-}/sys/devices/system/cpu/cpu$cf_cpu/cpufreq" 2>/dev/null && pwd); then
		echo cpufreq_policy=absent
		echo "cpufreq_line=CPU $cf_cpu has no cpufreq policy"
		return 0
	fi
	cf_dir=$cf_d
	printf 'cpufreq_policy=%s\ncpufreq_cur=%s\ncpufreq_min=%s\ncpufreq_max=%s\ncpufreq_steps=%s\ncpufreq_line=%s\n' \
		"${cf_d##*/}" "$(cf_read "$cf_d/scaling_cur_freq")" "$(cf_read "$cf_d/scaling_min_freq")" \
		"$(cf_read "$cf_d/scaling_max_freq")" "$(cf_read "$cf_d/scaling_available_frequencies")" \
		"$(cf_policy_line "$cf_d")"
}

# cf_set <policy directory> <min> <max>: writes scaling_min_freq and
# scaling_max_freq, min first unless the new min lies above the current max
# (then max first), so min <= max holds after each write. Prints one line
# per write; each value must read back. Returns 1 at the first write that
# fails or does not read back.
cf_set() {
	cf_cmax=$(cat "$1/scaling_max_freq" 2>/dev/null) || { echo "cpufreq: $1/scaling_max_freq unreadable"; return 1; }
	if [ "$2" -gt "$cf_cmax" ]; then cf_order="max min"; else cf_order="min max"; fi
	for cf_w in $cf_order; do
		if [ "$cf_w" = min ]; then cf_v=$2; else cf_v=$3; fi
		echo "cpufreq: write scaling_${cf_w}_freq $cf_v"
		if ! echo "$cf_v" 2>/dev/null >"$1/scaling_${cf_w}_freq"; then
			echo "cpufreq: the write of scaling_${cf_w}_freq $cf_v failed"
			return 1
		fi
		cf_got=$(cat "$1/scaling_${cf_w}_freq" 2>/dev/null)
		if [ "$cf_got" != "$cf_v" ]; then
			echo "cpufreq: scaling_${cf_w}_freq reads '$cf_got' after the write of $cf_v"
			return 1
		fi
	done
}

# cf_hold <normal|crash> <kHz> <tenths of a second>: holds the jump CPU's
# policy at the step: the facts first (cf_facts), then min and max set to
# it (cf_set), then scaling_cur_freq read until it is the step, at most the
# given time. Exit 0: held, "cpufreq_held=<scaling_cur_freq> <tenths>
# <cpuinfo_cur_freq>". Exit 3: refused before any write (the CPU or its
# policy unknown, the step not one of scaling_available_frequencies). Exit 1:
# not held, the old min and max written back ("cpufreq_restored=<min>
# <max>"); exit 2: not held, and that write-back failed too.
cf_hold() {
	cf_facts "$1"
	if [ -z "$cf_dir" ]; then
		echo "cpufreq: REFUSED: the jump CPU or its policy is not known ($cf_why)"
		return 3
	fi
	cf_steps=$(cf_read "$cf_dir/scaling_available_frequencies")
	case " $cf_steps " in
	*" $2 "*) ;;
	*)
		echo "cpufreq: REFUSED: $2 kHz is not one of ${cf_dir##*/}'s steps ($cf_steps)"
		return 3
		;;
	esac
	cf_omin=$(cat "$cf_dir/scaling_min_freq") && cf_omax=$(cat "$cf_dir/scaling_max_freq") || {
		echo "cpufreq: REFUSED: ${cf_dir##*/}'s scaling_min_freq or scaling_max_freq unreadable"
		return 3
	}
	echo "cpufreq_old=$cf_omin $cf_omax"
	cf_i=0
	if cf_set "$cf_dir" "$2" "$2"; then
		while :; do
			cf_cur=$(cat "$cf_dir/scaling_cur_freq" 2>/dev/null)
			[ "$cf_cur" != "$2" ] || {
				echo "cpufreq_held=$cf_cur $cf_i $(cf_read "$cf_dir/cpuinfo_cur_freq")"
				return 0
			}
			[ "$cf_i" -lt "$3" ] || break
			sleep 0.1
			cf_i=$((cf_i + 1))
		done
		echo "cpufreq: scaling_cur_freq reads '$cf_cur', not $2, $cf_i tenths of a second after the writes"
	fi
	if cf_set "$cf_dir" "$cf_omin" "$cf_omax"; then
		echo "cpufreq_restored=$cf_omin $cf_omax"
		return 1
	fi
	echo "cpufreq: the old scaling_min_freq $cf_omin and scaling_max_freq $cf_omax could not be written back"
	return 2
}

# cf_restore <policy name> <min> <max> <boot_id>: the hold's undo, only in
# the Linux that made it (its boot_id). Exit 0: written back
# ("cpufreq_restored=<min> <max>") or another Linux runs; 1: it failed.
cf_restore() {
	if [ "$(cat "${T6CF_ROOT:-}/proc/sys/kernel/random/boot_id" 2>/dev/null)" != "$4" ]; then
		echo "cpufreq: another Linux runs (boot_id changed): the hold ended with the one that made it"
		return 0
	fi
	cf_set "${T6CF_ROOT:-}/sys/devices/system/cpu/cpufreq/$1" "$2" "$3" || return 1
	echo "cpufreq_restored=$2 $3"
}
