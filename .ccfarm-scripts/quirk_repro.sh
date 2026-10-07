#!/bin/bash
# Minimal reproduction of the pi4-deqp-images.sh exit-status quirk: selected()
# pipes images() through a while whose body ends in a failed test when the
# last table line is not one of the requested labels; with set -o pipefail the
# function (and the script) then exit 1 although every requested image built.
set -uo pipefail
images() { printf 'A\t-\tf1\nB\t-\tf2\n'; }   # two labels; B is the last line
selected() {
	local want=" $* "
	images | while IFS=$'\t' read -r l c f; do
		[ $# -eq 0 ] || [[ $want == *" $l "* ]] && printf '%s\t%s\t%s\n' "$l" "$c" "$f"
	done
}
echo "selected A (last line B not selected):"
selected A
echo "  rc=$?"
echo "selected A B (last line selected):"
selected A B
echo "  rc=$?"
echo "selected all (no labels):"
selected
echo "  rc=$?"
