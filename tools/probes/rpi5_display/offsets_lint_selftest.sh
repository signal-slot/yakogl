#!/bin/bash
# offsets_lint_selftest.sh - self-test of offsets_lint.sh on fixture headers
# written to a temporary directory under $TMPDIR.
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
lint="$here/offsets_lint.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/offsets_lint_st.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT

total=0
failed=0
check() { # name, condition result (0 = pass)
	total=$((total + 1))
	if [ "$2" -ne 0 ]; then
		failed=$((failed + 1))
		echo "FAIL: $1"
	fi
}
# run <dir-name> <expected rc> <expected last line>; sets out
run() {
	out=$(LC_ALL=C bash "$lint" "$tmp/$1"/*.h 2>&1)
	rc=$?
}

# agree: same values, written differently (hex/decimal/shift, prefixes)
mkdir "$tmp/agree"
cat > "$tmp/agree/a.h" <<'EOF'
#define YD5_HVS6_VERSION 0x00000000u
#define YD5_HVS6_CTRL0_ENB (1u << 31)
#define YD5_HVS6C_DISP0_CTRL0 0x00000030u /* comment */
#define YD5_HVS6_FWIDTH_SHIFT 16u
EOF
cat > "$tmp/agree/b.h" <<'EOF'
/* multi-line
 * comment #define HVS6_VERSION 5 */
#define HVS6_VERSION 0x0000u
#define HVS6_CTRL0_ENB 2147483648
#define HVS6C_DISP0_CTRL0 48
enum { HVS6_FWIDTH_SHIFT = 0x10 };
EOF
run agree
check "agree: exit 0" "$rc"
[ "$out" = "offsets_lint: 2 headers, 4 shared names, 0 conflicts" ]
check "agree: summary line" $?

# conflict: one name, two values; one block+offset, two names
mkdir "$tmp/conflict"
cat > "$tmp/conflict/a.h" <<'EOF'
#define YD5_PV_CTRL 0x0010u
#define YD5_PV_STAT 0x0014u
EOF
cat > "$tmp/conflict/b.h" <<'EOF'
#define PV_CTRL 0x0018u
static const unsigned PV_STATUS = 0x0014u;
EOF
run conflict
check "conflict: exit 1" "$([ "$rc" -eq 1 ]; echo $?)"
echo "$out" | grep -qx 'offsets_lint: CTRL a.h=0x10 b.h=0x18'
check "conflict: name line" $?
echo "$out" | grep -q 'offsets_lint: PV+0x14 .*=.*\.h .*=.*\.h'
check "conflict: block+offset line" $?
echo "$out" | tail -n 1 | grep -qx 'offsets_lint: 2 headers, 1 shared names, 2 conflicts'
check "conflict: summary line" $?

# prefix normalisation: different prefixes, the same name, one value
mkdir "$tmp/prefix"
cat > "$tmp/prefix/a.h" <<'EOF'
#define YD5_HVS6_STATUS_MODE_RUN 2u
#define YD5_HVS6C_DISP0_DL 0x0048u
EOF
cat > "$tmp/prefix/b.h" <<'EOF'
#define HVS6_STATUS_MODE_RUN 3u
#define HVS6C_DISP0_DL 0x0048u
EOF
run prefix
check "prefix: exit 1" "$([ "$rc" -eq 1 ]; echo $?)"
echo "$out" | grep -qx 'offsets_lint: STATUS_MODE_RUN a.h=0x2 b.h=0x3'
check "prefix: YD5_HVS6_ and HVS6_ share a name" $?
echo "$out" | tail -n 1 | grep -qx 'offsets_lint: 2 headers, 2 shared names, 1 conflicts'
check "prefix: HVS6C_ stays its own name, summary" $?

# hex versus decimal of the same value, enum counters, suffixes, non-numeric skipped
mkdir "$tmp/hexdec"
cat > "$tmp/hexdec/a.h" <<'EOF'
#define X_MASK 0xFFu
#define X_OR (1u << 3 | 1u << 1)
#define X_NEG (-22)
#define X_SKIP sizeof(int)
#define X_FN(a) ((a) + 1)
enum e { E_A, E_B, E_C = 7, E_D };
EOF
cat > "$tmp/hexdec/b.h" <<'EOF'
#define X_MASK 255
#define X_OR 10UL
#define X_NEG -22
#define X_SKIP 4
static const uint32_t E_D = 8u;
static const int E_B = 1;
EOF
run hexdec
check "hexdec: exit 0" "$rc"
echo "$out" | tail -n 1 | grep -qx 'offsets_lint: 2 headers, 5 shared names, 0 conflicts'
check "hexdec: five shared names, no conflict" $?

# usage errors
bash "$lint" -x >/dev/null 2>&1
check "usage: unknown option exits 2" "$([ $? -eq 2 ]; echo $?)"
bash "$lint" "$tmp/nonexistent.h" >/dev/null 2>&1
check "usage: missing file exits 2" "$([ $? -eq 2 ]; echo $?)"

echo "offsets_lint_selftest: $((total - failed)) of $total checks, $failed failed"
[ "$failed" -eq 0 ]
