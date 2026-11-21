#!/bin/bash
# Regenerates the fixture trees (they are committed; this documents them).
cd "$(dirname "$0")" || exit 1
mk() { # name kconfig-default-line cited-dirs existing-dirs
    d=$1
    rm -rf "$d"
    mkdir -p "$d/driver/zephyr" "$d/docs/notes"
    printf 'config OTHER\n\tdefault y\n\nconfig YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER\n\tbool "x"\n%s\n\nconfig NEXT\n\tdefault y\n' "$2" >"$d/driver/zephyr/Kconfig"
    {
        printf '# t\n\n## Design: the firmware'\''s partial render (PR)\n\n**Status, 2026-11-21.**\n- Measured:\n'
        for c in $3; do printf '  - `tools/cts/log/%s/` ran.\n' "$c"; done
        printf '\n**The knob:** see `tools/cts/log/elsewhere/`.\n'
    } >"$d/docs/notes/pvr-render-pool.md"
    for c in $4; do
        mkdir -p "$d/tools/cts/log/$c"
        echo r >"$d/tools/cts/log/$c/README"
    done
}
mk pass '	default n' "a b" "a b"
mk defaulty '	default y' "a" "a"
mk nocite '' "" ""
mk missing '' "a gone" "a"
