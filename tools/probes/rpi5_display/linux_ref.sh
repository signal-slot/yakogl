#!/bin/bash
# Read-only dump of the Raspberry Pi 5 scanout state under stock Linux.
# Usage: linux_ref.sh [OUTDIR]   (default: mktemp -d under $TMPDIR)
# Writes only OUTDIR/linux_ref.txt. See linux_ref.md.
# ROOT (env) prefixes /proc, /sys and /dev/mem; it is for the host self-test.

ROOT=${ROOT:-}
OUT=${1:-}
if [ -z "$OUT" ]; then
	OUT=$(mktemp -d "${TMPDIR:-/tmp}/linux_ref.XXXXXX") || exit 1
fi
mkdir -p "$OUT" || exit 1
RES="$OUT/linux_ref.txt"
: >"$RES" || exit 1

HVS_BASE=0x107c580000
PV0_BASE=0x107c410000
NFACTS=0
NSKIP=0

# Strip anything that looks like an IP, MAC or serial number.
redact() {
	sed -E \
		-e 's/([0-9a-fA-F]{1,2}[:-]){5}[0-9a-fA-F]{1,2}/<mac>/g' \
		-e 's/[0-9]{1,3}(\.[0-9]{1,3}){3}/<ip>/g' \
		-e 's/([0-9a-fA-F]{0,4}:){3,7}[0-9a-fA-F]{0,4}/<ip6>/g' \
		-e 's/([Ss]erial[^:=]*[:=] *)[^ ]+/\1<redacted>/g'
}

emit() { # key value
	local line
	line=$(printf '%s: %s' "$1" "$2" | tr -d '\000' | redact)
	printf '%s\n' "$line"
	printf '%s\n' "$line" >>"$RES"
	NFACTS=$((NFACTS + 1))
}

skip() { # reason
	local line
	line=$(printf 'skip: %s' "$1" | redact)
	printf '%s\n' "$line"
	printf '%s\n' "$line" >>"$RES"
	NSKIP=$((NSKIP + 1))
}

# ---- vcgencmd (read-only queries only) -------------------------------
VCG=$(command -v vcgencmd 2>/dev/null)
if [ -n "$VCG" ] && [ -z "$ROOT" ]; then
	emit get_throttled "$("$VCG" get_throttled 2>&1 | head -n 1)"
	emit vcgencmd_version "$("$VCG" version 2>&1 | tr '\n' ' ')"
	emit display_power "$("$VCG" display_power 2>&1 | head -n 1)"
else
	skip "vcgencmd not present (get_throttled, version, display_power)"
fi

# ---- device tree ------------------------------------------------------
DT="$ROOT/proc/device-tree"

dt_str() { tr '\000' ' ' <"$1" 2>/dev/null | sed 's/ *$//'; }

dt_words() { # file -> one hex word (no 0x) per line
	od -An -v -tx1 -w4 "$1" 2>/dev/null | tr -d ' '
}

dt_u32s() { # file -> space separated decimal big-endian words
	local w out=""
	for w in $(dt_words "$1"); do
		out="$out $((16#$w))"
	done
	printf '%s' "${out# }"
}

dt_hexcells() { # file -> 0x-prefixed words
	local w out=""
	for w in $(dt_words "$1"); do
		out="$out 0x$w"
	done
	printf '%s' "${out# }"
}

HDMI0_NODE=""
if [ -d "$DT" ]; then
	nodes=$(find "$DT" -maxdepth 4 -type d \( -name 'hvs*' -o -name 'pixelvalve*' -o -name 'hdmi*' \
		-o -name 'framebuffer*' -o -name 'gpu' -o -name 'vc4*' \) 2>/dev/null | sort)
	if [ -z "$nodes" ]; then
		skip "no display nodes in device tree"
	fi
	for n in $nodes; do
		rel=${n#"$DT"}
		rel=${rel#/}
		[ -z "$rel" ] && continue
		k="dt.$rel"
		if [ -f "$n/status" ]; then
			emit "$k.status" "$(dt_str "$n/status")"
		else
			emit "$k.status" "okay (no status property)"
		fi
		[ -f "$n/compatible" ] && emit "$k.compatible" "$(dt_str "$n/compatible")"
		[ -f "$n/reg" ] && emit "$k.reg" "$(dt_hexcells "$n/reg")"
		[ -f "$n/reg-names" ] && emit "$k.reg-names" "$(dt_str "$n/reg-names")"
		for p in width height stride; do
			[ -f "$n/$p" ] && emit "$k.$p" "$(dt_u32s "$n/$p")"
		done
		[ -f "$n/format" ] && emit "$k.format" "$(dt_str "$n/format")"
		case "${n##*/}" in
		hdmi@*)
			if [ -z "$HDMI0_NODE" ] && [ -f "$n/reg" ] && [ -f "$n/reg-names" ]; then
				HDMI0_NODE=$n
			fi
			;;
		esac
	done
else
	skip "no device tree at /proc/device-tree"
fi

# ---- DRM and framebuffer sysfs ---------------------------------------
if [ -d "$ROOT/sys/class/drm" ]; then
	for c in "$ROOT"/sys/class/drm/card*-*; do
		[ -e "$c" ] || continue
		b=${c##*/}
		for p in status enabled dpms modes; do
			[ -f "$c/$p" ] && emit "drm.$b.$p" "$(head -n 3 "$c/$p" 2>/dev/null | tr '\n' ' ')"
		done
	done
else
	skip "no /sys/class/drm"
fi

fbn=0
for f in "$ROOT"/sys/class/graphics/fb*; do
	[ -d "$f" ] || continue
	b=${f##*/}
	fbn=1
	for p in name virtual_size bits_per_pixel stride; do
		[ -f "$f/$p" ] && emit "fb.$b.$p" "$(head -n 1 "$f/$p" 2>/dev/null)"
	done
done
[ $fbn -eq 0 ] && skip "no /sys/class/graphics/fb*"

# ---- debugfs register dumps ------------------------------------------
DBG="$ROOT/sys/kernel/debug/dri"
HAVE_DBG_REGS=0
if [ -d "$DBG" ]; then
	for d in "$DBG"/*/; do
		[ -d "$d" ] || continue
		dn=$(basename "$d")
		case "$dn" in [0-9]*) ;; *) continue ;; esac
		for f in "$d"hvs_* "$d"*regs* "$d"hdmi* "$d"pixelvalve*; do
			[ -f "$f" ] || continue
			b=${f##*/}
			if [ ! -r "$f" ]; then
				skip "debugfs $dn/$b not readable"
				continue
			fi
			i=0
			while IFS= read -r line; do
				i=$((i + 1))
				emit "debugfs.$dn.$b[$i]" "$line"
			done < <(head -n 64 "$f" 2>/dev/null)
			case "$b" in *regs*) HAVE_DBG_REGS=1 ;; esac
		done
	done
else
	skip "debugfs dri not mounted or not readable"
fi

# ---- /proc/iomem display lines ---------------------------------------
if [ -r "$ROOT/proc/iomem" ]; then
	n=0
	while IFS= read -r line; do
		n=$((n + 1))
		emit "iomem[$n]" "$line"
	done < <(grep -i -E 'hvs|pixelvalve|hdmi|vc4|crtc|framebuffer|simple-fb|v3d' "$ROOT/proc/iomem" 2>/dev/null | sed 's/^ *//')
	[ $n -eq 0 ] && skip "no display lines in /proc/iomem"
else
	skip "/proc/iomem not readable"
fi

# ---- loaded modules (read only) --------------------------------------
if [ -r "$ROOT/proc/modules" ]; then
	emit modules "$(awk '{print $1}' "$ROOT/proc/modules" | grep -E 'vc4|v3d|drm|hdmi|etnaviv|simplefb|fb' | tr '\n' ' ')"
else
	skip "/proc/modules not readable"
fi

# ---- /dev/mem words, only when no debugfs register dump --------------
MEM="$ROOT/dev/mem"

# rd_words NAME BASE NWORDS: read 32-bit words with a read-only helper.
rd_words() {
	local name=$1 base=$2 n=$3 i v out
	if [ -n "$ROOT" ]; then
		# self-test: the fake /dev/mem is a plain file indexed by the low 24 bits
		out=$(python3 -I -c '
import sys
path, base, n = sys.argv[1], int(sys.argv[2], 0), int(sys.argv[3])
with open(path, "rb") as f:
    for i in range(n):
        f.seek((base + 4 * i) & 0xffffff)
        d = f.read(4)
        if len(d) < 4:
            break
        print("0x%08x" % int.from_bytes(d, "little"))
' "$MEM" "$base" "$n" 2>&1) || out="ERR $out"
	elif command -v python3 >/dev/null 2>&1; then
		out=$(python3 -I -c '
import sys, mmap, os
path, base, n = sys.argv[1], int(sys.argv[2], 0), int(sys.argv[3])
page = mmap.PAGESIZE
off = base & ~(page - 1)
rel = base - off
span = (rel + 4 * n + page - 1) // page * page
fd = os.open(path, os.O_RDONLY)
m = mmap.mmap(fd, span, mmap.MAP_SHARED, mmap.PROT_READ, offset=off)
for i in range(n):
    print("0x%08x" % int.from_bytes(m[rel + 4 * i:rel + 4 * i + 4], "little"))
' "$MEM" "$base" "$n" 2>&1) || out="ERR $out"
	elif command -v busybox >/dev/null 2>&1; then
		out=""
		for ((i = 0; i < n; i++)); do
			# no value argument: a read
			v=$(busybox devmem $((base + 4 * i)) 32 2>/dev/null) || break
			out="$out$v"$'\n'
		done
	else
		skip "no read-only helper (python3 or busybox devmem) for $name"
		return
	fi
	case "$out" in
	"" | ERR*)
		skip "$name read failed"
		return
		;;
	esac
	i=0
	while IFS= read -r v; do
		emit "mem.$name[$(printf '0x%x' $((i * 4)))]" "$v"
		i=$((i + 1))
	done <<<"$out"
}

# hd_base NODE: address of the "hd" reg entry (address cells 2, size cells
# deduced from the reg length and the number of reg-names).
hd_base() {
	local node=$1 names nreg cells per idx nm hi lo w
	names=$(dt_str "$node/reg-names")
	nreg=$(printf '%s\n' "$names" | wc -w)
	cells=$(($(wc -c <"$node/reg") / 4))
	[ "$nreg" -gt 0 ] && [ $((cells % nreg)) -eq 0 ] || return 1
	per=$((cells / nreg))
	idx=0
	for nm in $names; do
		if [ "$nm" = hd ]; then
			w=$(dt_words "$node/reg" | tr '\n' ' ')
			# shellcheck disable=SC2086
			set -- $w
			hi=${*:$((idx * per + 1)):1}
			lo=${*:$((idx * per + 2)):1}
			printf '0x%s%s' "$hi" "$lo"
			return 0
		fi
		idx=$((idx + 1))
	done
	return 1
}

if [ "$HAVE_DBG_REGS" -eq 1 ]; then
	skip "/dev/mem not read: debugfs register dump present"
elif [ ! -r "$MEM" ]; then
	skip "/dev/mem absent or not readable"
else
	rd_words hvs "$HVS_BASE" 64
	rd_words pixelvalve0 "$PV0_BASE" 16
	hd=""
	[ -n "$HDMI0_NODE" ] && hd=$(hd_base "$HDMI0_NODE")
	if [ -n "$hd" ]; then
		rd_words hdmi0_hd "$hd" 16
	else
		skip "hdmi0 hd region address not found in device tree"
	fi
fi

line="linux_ref: done ($NFACTS facts, $NSKIP skipped)"
printf '%s\n' "$line"
printf '%s\n' "$line" >>"$RES"
exit 0
