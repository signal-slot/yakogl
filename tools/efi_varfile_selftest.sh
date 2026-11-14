#!/usr/bin/env bash
# Host selftest of tools/efi_varfile.py (no board). Synthetic ubootefi.var
# files are built here in the writer's format: header and entry layout from
# include/efi_variable.h:114-137, writer lib/efi_loader/efi_var_file.c:56-118
# (entries 8-byte aligned :104, length incl. header :110/:113, crc32 over
# the entries :111-112). Counter: lib/fwu_updates/fwu.c:42-45 (u16
# TrialStateCtr, EFI global GUID).
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/efi_varfile.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/efivar.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=1; }

python3 -I - "$tmp" <<'PY' || exit 1
import struct, sys, zlib, uuid
d = sys.argv[1]
MAGIC = 0x0161566966456255
GLOBAL = uuid.UUID("8be4df61-93ca-11d2-aa0d-00e098032b8c").bytes_le
OTHER = uuid.UUID("12345678-1234-1234-1234-123456789abc").bytes_le
def entry(name, guid, attr, data):
    e = struct.pack("<IIQ", len(data), attr, 0) + guid
    e += name.encode("utf-16-le") + b"\0\0" + data
    return e + b"\0" * (-len(e) % 8)
def image(entries):
    body = b"".join(entries)
    return struct.pack("<QQII", 0, MAGIC, 24 + len(body),
                       zlib.crc32(body)) + body
def put(n, b):
    with open(d + "/" + n, "wb") as f:
        f.write(b)
NV_BS = 0x3
other = entry("PlatformLang", OTHER, 0x3, b"en-US\0")
ctr = entry("TrialStateCtr", GLOBAL, NV_BS, struct.pack("<H", 2))
good = image([other, ctr])
put("good_ctr.var", good)
put("good_noctr.var", image([other]))
put("empty.var", image([]))
g = bytearray(good); g[-4] ^= 0xFF  # flip a data byte (2 data + 2 pad), crc stays
put("badcrc.var", bytes(g))
put("trunc.var", good[:-9])
put("short.var", good[:10])
m = bytearray(good); m[8] ^= 1
put("badmagic.var", bytes(m))
# same name under another GUID is not the counter
put("wrongguid.var",
    image([entry("TrialStateCtr", OTHER, NV_BS, struct.pack("<H", 1))]))
PY

run() { out=$(python3 -I "$tool" "$@" 2>&1); rc=$?; }
expect() { # name rc substr args...
    local n=$1 erc=$2 sub=$3; shift 3
    run "$@"
    if [ "$rc" = "$erc" ] && [[ "$out" == *"$sub"* ]]; then ok "$n"
    else bad "$n (rc=$rc, out=$out)"; fi
}

expect "good file lists vars" 0 "PlatformLang" "$tmp/good_ctr.var"
expect "list shows counter" 0 "TrialStateCtr" --list "$tmp/good_ctr.var"
expect "counter present" 0 "fwu-trial: counter 2" --fwu "$tmp/good_ctr.var"
expect "counter absent" 0 "fwu-trial: absent" --fwu "$tmp/good_noctr.var"
expect "empty file, absent" 0 "fwu-trial: absent" --fwu "$tmp/empty.var"
expect "same name other GUID absent" 0 "fwu-trial: absent" --fwu "$tmp/wrongguid.var"
expect "bad crc" 2 "efivar: bad file: crc32 mismatch" --fwu "$tmp/badcrc.var"
expect "truncated file" 2 "efivar: bad file: truncated" --fwu "$tmp/trunc.var"
expect "shorter than header" 2 "efivar: bad file: truncated" "$tmp/short.var"
expect "bad magic" 2 "efivar: bad file: bad magic" "$tmp/badmagic.var"
expect "missing file" 2 "efivar: bad file:" "$tmp/nonexistent.var"
expect "usage" 64 "usage"

# read-only: the inputs are byte-identical after the runs, and the tool
# opens files only in 'rb' mode
before=$(sha256sum "$tmp"/*.var)
python3 -I "$tool" --fwu "$tmp/good_ctr.var" >/dev/null
[ "$before" = "$(sha256sum "$tmp"/*.var)" ] && ok "inputs untouched" || bad "inputs changed"
if grep -n 'open(' "$tool" | grep -v '"rb"' | grep -q .; then bad "non-rb open"; else ok "only rb opens"; fi

[ $fail = 0 ] && echo "efi_varfile selftest: PASS" || echo "efi_varfile selftest: FAIL"
exit $fail
