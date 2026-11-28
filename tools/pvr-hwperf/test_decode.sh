#!/bin/sh
# Self-test: decode fixture.log and compare with expected.txt.
d=$(cd "$(dirname "$0")" && pwd)
out=$(python3 -I "$d/decode.py" "$d/fixture.log") || exit 1
printf '%s\n' "$out" | diff -u "$d/expected.txt" - || { echo "test_decode: FAIL"; exit 1; }
echo "test_decode: ok"
