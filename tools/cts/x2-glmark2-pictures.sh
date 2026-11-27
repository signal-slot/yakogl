#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The i.MX8MP glmark2 picture session (issue #17) on the Armadillo-X2, which
# stands in for the away Verdin iMX8MP (farm rule of 2026-11-27): the 33
# scenes of the imx8mp column of tools/cts/glmark2-pictures.txt, one image a
# scene (the X2 has no runtime scene selector: "-b <scene>" is baked in through
# CONFIG_GLMARK2_ARGS), one kexec-dev jump a scene, a screenshot of the panel
# (camera or HDMI KVM) while the image holds its last frame.
#
#   tools/cts/x2-glmark2-pictures.sh [--dry-run] list
#   tools/cts/x2-glmark2-pictures.sh [--dry-run] [--commit <rev>] build <out-dir> [SCENE...]
#   tools/cts/x2-glmark2-pictures.sh [--dry-run] run <img-dir> <record-dir> [SCENE...]
#
# SCENE      NN (1..33, as list numbers them) or a whole label; all when none.
# list       "<label> <scene info string>" per scene, in the column's order.
#            label = gmNN-<slug>: NN two digits, slug the info string
#            lowercased, every run of non-[a-z0-9] turned into '-', stripped
#            of '-', cut at 48 characters (the photograph slug of
#            imx8mp-glmark2-record-check.py and glmark2-fb.py), so a label
#            holds only [a-z0-9-] and is a safe directory name. The column
#            must hold 33 scenes (else exit 2).
# build      per scene, from <rev> (default HEAD), which must be an ancestor of
#            origin/main (board images are built only from commits on main;
#            else exit 3, before anything is built; HEAD with tracked changes
#            is refused too):
#              (cd $ZEPHYR_WS && west build -p always -b armadillo_x2/mimx8ml8/a53
#                -d <out-dir>/build-<label> <src>/driver/samples/zephyr/glmark2 --
#                -DZEPHYR_MODULES=<zephyr-modules.sh> -DGLMARK2_SRC=$GLMARK2_SRC
#                '-DCONFIG_GLMARK2_ARGS="-b <scene>"')
#            (the command of x2-glmark2-scenes.sh build_image()), with the
#            build directory named after the label, never the scene: a ':' in
#            a build path breaks cargo's LD_LIBRARY_PATH join, and 31 of the
#            33 scenes have one. <scene> is glmark2's -b form of the info
#            string ("[build] use-vbo=false" -> build:use-vbo=false,
#            "[jellyfish] <default>" -> jellyfish), with each ';' of an
#            effect2d kernel written '\;' (unescaped, CMake splits the value
#            as a list and the image runs another kernel). After the build the
#            .config line must read CONFIG_GLMARK2_ARGS="-b <scene>" exactly
#            and the image must carry the watchdog marker 'gc_wdog_arm: armed'
#            (board.sh x2 refuses an image without it); then only
#            <out-dir>/<label>/zephyr.bin and <out-dir>/<label>/zephyr.bin.sha256
#            (sha256sum's form) are kept and the build tree is removed (disk
#            is tight: one tree is a few GB). A failed build keeps
#            <out-dir>/<label>.log. <out-dir>/commit holds the 40-hex commit;
#            a label whose image and sha256 are there and agree, from the same
#            commit, is kept, not rebuilt; another commit in <out-dir> is
#            refused (exit 3). An <out-dir> path with ':', ';' or blanks is
#            refused. A --commit other than HEAD is checked out into
#            <out-dir>/tree-<sha8> (git worktree), removed at the end.
#            One line a scene: "<label> sha256=<64 hex> commit=<sha8>".
# run        per scene, in order:
#              X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev tools/board.sh x2 run \
#                <img-dir>/<label>/zephyr.bin <record>/runs <label>
#            (NETBOOT_NO_WATCHDOG and SEL unset; BOARD_HOST and BOARD_UART
#            from the environment only, never a file). board.sh captures the
#            console (BOARD_UART) into its run directory
#            (<record>/runs/<YYYYMMDD-HHMM>-<label>-<image sha8>/console.txt,
#            or console.raw from a serial bridge): this script reads that live
#            capture and never opens the UART itself (board.sh refuses a
#            device somebody holds, and a bridge takes one client). On the
#            first 'board: holding' line (driver/zephyr/lib/board/common/
#            board.c, CONFIG_YAKOGL_BOARD_HOLD_S=20 s after the 10 s scene)
#            it runs `$X2_SHOT_CMD <record>/photos/NN-<slug>.png` (X2_SHOT_CMD
#            is split into words; the command writes the PNG), or, when
#            X2_SHOT_CMD is unset, prints "shot-now NN <slug>" and waits
#            X2_SHOT_WAIT seconds (default 15) for a person to take it. After
#            board.sh ends, the scene's "glmark2-picture: <info>: ..." line of
#            the capture goes into <record>/console.txt (a line for the same
#            scene from an earlier run of this record is replaced) and one
#            line into <record>/scenes.txt:
#              NN <label> rc=<board.sh rc> holding=yes|no shot=<state> picture=yes|no
#            board.sh's output is shown and kept as <record>/runs/<label>.out,
#            with this host's paths replaced by <record>, <img-dir>, <repo>, ~.
#            A board that does not come back (board.sh rc 5 or 6) or a refused
#            run (rc 3) stops the session (exit 5 / 3, no retry); any other
#            not-done run (rc 4) is a line and the next scene goes on.
#            Finally imx8mp-glmark2-record-check.py's verdict on the record is
#            printed (it is not the exit status).
#            Before anything: BOARD_UART must be set (it is the hold cue and
#            the picture lines); <img-dir>/commit must be an ancestor of
#            origin/main (exit 3) and every selected image must match its
#            sha256 file (exit 3).
#            The record: <record-dir> itself when its name is
#            <YYYYMMDD-HHMM>-eye-<sha8> (a session resumed: its meta.txt must
#            name the same commit and images, else exit 3), otherwise a new
#            <record-dir>/<YYYYMMDD-HHMM>-eye-<sha8>/ (records/imx8mp/ is the
#            place, gitignored). sha8 is the first 8 digits of the sha256 of
#            <record>/images.sha256, the manifest "<sha256>  <label>" of every
#            image of <img-dir>: 33 images, one name. meta.txt:
#              board=x2
#              board_measured=Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the Verdin iMX8MP
#              commit=<40 hex>
#              route=x2-kexec-dev, hold cue 'board: holding' on the console
#              image_sha256_<label>=<64 hex>      (one per image)
#              image_sha256=<sha256 of images.sha256>   (last: the name's sha8)
#            the layout tools/cts/imx8mp-glmark2-record-check.py checks
#            (console lines, photos/NN-<slug>.png, sha8, board_measured,
#            redaction).
# --dry-run  list: as without. build: print the west line of each scene
#            (each word shell-quoted, so it pastes as it is) and build
#            nothing (no worktree, no zephyr-modules.sh; an unset ZEPHYR_WS or
#            GLMARK2_SRC is shown as <ZEPHYR_WS>, <GLMARK2_SRC>). run: create
#            no record, print the board.sh line of each scene, run board.sh
#            with --dry-run (it touches no board), print the shot hook call
#            it would make, and read no console.
#
# Environment:
#   ZEPHYR_WS (or ZEPHYR_WORKSPACE), GLMARK2_SRC, WEST_VENV (or ZEPHYR_VENV),
#                    ZEPHYR_SDK_INSTALL_DIR: build; each one unset is taken
#                    from `tools/cts/deps-env.sh --present imx8mp` (the
#                    project's dependency layout)
#   BOARD_HOST, BOARD_UART   run: read by board.sh (BOARD_UART also required here)
#   X2_SHOT_CMD, X2_SHOT_WAIT   run: the screenshot hook, the manual wait (above)
#   X2P_REPO         the checkout (default: this script's)
#   X2P_MAIN_REF     the ref commits must be on (default origin/main)
#   X2P_PICTURES     the scene list (default tools/cts/glmark2-pictures.txt)
#   X2P_BOARD_SH, X2P_WEST, X2P_MODULES_SH, X2P_DEPS_ENV, X2P_RECORD_CHECK
#                    board.sh, west, zephyr-modules.sh, deps-env.sh and the
#                    record check (defaults: the checkout's; the self-test,
#                    tools/cts/x2-glmark2-pictures-selftest.sh, puts stubs here)
#   X2P_POLL         run: seconds between reads of the capture (default 1)
#
# Exit: 0 done; 2 usage or environment; 3 refused; 4 a build failed or a scene
# has no picture line; 5 the board did not come back.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=${X2P_REPO:-$(cd "$HERE/../.." && pwd)}
MAIN_REF=${X2P_MAIN_REF:-origin/main}
PICTURES=${X2P_PICTURES:-$REPO/tools/cts/glmark2-pictures.txt}
BOARD_SH=${X2P_BOARD_SH:-$REPO/tools/board.sh}
WEST=${X2P_WEST:-west}
MODULES_SH=${X2P_MODULES_SH:-$HERE/zephyr-modules.sh}
DEPS_ENV=${X2P_DEPS_ENV:-$HERE/deps-env.sh}
RECORD_CHECK=${X2P_RECORD_CHECK:-$HERE/imx8mp-glmark2-record-check.py}
POLL=${X2P_POLL:-1}
ZBOARD=armadillo_x2/mimx8ml8/a53
MARKER='gc_wdog_arm: armed'
HOLD_RE='board: holding'
NSCENES=33
MEASURED='Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the Verdin iMX8MP'

die() { echo "x2-glmark2-pictures: $1" >&2; exit "${2:-2}"; }
say() { echo "x2-glmark2-pictures: $*"; }

DRY=0 COMMIT=HEAD
while [ $# -gt 0 ]; do
    case $1 in
    --dry-run) DRY=1; shift ;;
    --commit) [ $# -ge 2 ] || die "--commit needs a revision"; COMMIT=$2; shift 2 ;;
    -h | --help) sed -n '5,126p' "$0"; exit 0 ;;
    -*) die "unknown option $1" ;;
    *) break ;;
    esac
done
[ $# -ge 1 ] || die "usage: x2-glmark2-pictures.sh [--dry-run] list | [--commit <rev>] build <out-dir> [SCENE...] | run <img-dir> <record-dir> [SCENE...]"
VERB=$1
shift

# ---- the scenes -------------------------------------------------------------------
# rows: "<label>TAB<-b argument>TAB<info string>" per scene of the imx8mp column
rows() {
    awk -F' [|] ' '$1 == "imx8mp" {
        info = $2; n++
        s = tolower(info); gsub(/[^a-z0-9]+/, "-", s); sub(/^-+/, "", s); sub(/-+$/, "", s)
        s = substr(s, 1, 48)
        b = info; sub(/^\[/, "", b); sub(/\].*/, "", b)
        o = info; sub(/^\[[^]]*\] */, "", o)
        arg = (o == "<default>" || o == "") ? b : b ":" o
        printf "gm%02d-%s\t-b %s\t%s\n", n, s, arg, info
    }' "$PICTURES"
}
ROWS=$(rows) || die "cannot read $PICTURES"
[ "$(grep -c . <<< "$ROWS")" = "$NSCENES" ] ||
    die "$PICTURES: the imx8mp column has $(grep -c . <<< "$ROWS") scenes, not $NSCENES"

# select SCENE...: the selected rows (all when none), in the column's order
SEL_ROWS=
select_rows() {
    local s want=() l a i k
    if [ $# -eq 0 ]; then SEL_ROWS=$ROWS; return; fi
    for s in "$@"; do
        if [[ $s =~ ^[0-9]{1,2}$ ]] && [ "$((10#$s))" -ge 1 ] && [ "$((10#$s))" -le $NSCENES ]; then
            want+=("$(printf 'gm%02d' "$((10#$s))")")
        elif grep -q "^$s	" <<< "$ROWS" && [[ $s =~ ^gm[0-9]{2}-[a-z0-9-]+$ ]]; then
            want+=("${s:0:4}")
        else
            die "no scene '$s' (NN 1..$NSCENES or a label of 'list')"
        fi
    done
    SEL_ROWS=$(while IFS=$'\t' read -r l a i; do
        for k in "${want[@]}"; do [ "${l:0:4}" = "$k" ] && { printf '%s\t%s\t%s\n' "$l" "$a" "$i"; break; }; done
    done <<< "$ROWS")
}

# squote WORD...: the words, each quoted for a shell when it needs it
squote() {
    local w out=()
    for w in "$@"; do
        if [[ $w =~ ^[A-Za-z0-9_./=:,+@%-]+$ ]]; then out+=("$w"); else out+=("'${w//\'/\'\\\'\'}'"); fi
    done
    echo "${out[*]}"
}

# on_main SHA: SHA is an ancestor of MAIN_REF, or a refusal (exit 3)
on_main() {
    git -C "$REPO" rev-parse --verify -q "$MAIN_REF^{commit}" > /dev/null ||
        die "no $MAIN_REF in $REPO: cannot show that $1 is on it" 3
    git -C "$REPO" merge-base --is-ancestor "$1" "$MAIN_REF" 2> /dev/null ||
        die "refused: $1 is not an ancestor of $MAIN_REF (only images from a commit on main run on the board)" 3
}

# ---- list -------------------------------------------------------------------------
cmd_list() {
    [ $# -eq 0 ] || die "usage: x2-glmark2-pictures.sh list"
    local l a i
    while IFS=$'\t' read -r l a i; do echo "$l $i"; done <<< "$ROWS"
}

# ---- build ------------------------------------------------------------------------
# deps_fill: the build's variables that are unset, from deps-env.sh --present imx8mp
deps_fill() {
    local k v
    while IFS='=' read -r k v; do
        case $k in
        ZEPHYR_WS | ZEPHYR_WORKSPACE | GLMARK2_SRC | WEST_VENV | ZEPHYR_VENV | ZEPHYR_SDK_INSTALL_DIR)
            [ -n "${!k:-}" ] || export "$k=$v" ;;
        esac
    done < <("$DEPS_ENV" --present imx8mp 2> /dev/null)
}

# west_words LABEL ARG: the west command of one scene, into WORDS
WORDS=()
west_words() {
    local a=${2//;/\\;}
    WORDS=(build -p always -b "$ZBOARD" -d "$OUT/build-$1" "$SRC/driver/samples/zephyr/glmark2" --
        "-DZEPHYR_MODULES=$MODS" "-DGLMARK2_SRC=${GLMARK2_SRC:-<GLMARK2_SRC>}"
        "-DCONFIG_GLMARK2_ARGS=\"$a\"")
}

cmd_build() {
    [ $# -ge 1 ] || die "usage: x2-glmark2-pictures.sh [--dry-run] [--commit <rev>] build <out-dir> [SCENE...]"
    OUT=$1
    shift
    select_rows "$@"
    [[ $OUT =~ [:\;[:space:]] ]] && die "out-dir '$OUT': no ':', ';' or blanks (cargo joins build paths into LD_LIBRARY_PATH)"
    local sha short l a i b want cfg got rc=0 tree= venv ws
    sha=$(git -C "$REPO" rev-parse --verify -q "$COMMIT^{commit}") || die "no commit $COMMIT in $REPO" 3
    on_main "$sha"
    short=${sha:0:8}
    if [ "$sha" = "$(git -C "$REPO" rev-parse HEAD)" ]; then
        git -C "$REPO" diff --quiet HEAD -- ||
            die "refused: tracked changes in $REPO; the images would not be the commit $short" 3
        SRC=$REPO
    else
        SRC='<out-dir>/tree-'$short
    fi
    [ "$DRY" = 1 ] || deps_fill
    ws=${ZEPHYR_WS:-${ZEPHYR_WORKSPACE:-}}
    if [ "$DRY" = 1 ]; then
        MODS=$([ -n "$ws" ] && echo "<zephyr-modules.sh $ws>" || echo "<zephyr-modules>")
        [ "$SRC" = "$REPO" ] || { SRC=$OUT/tree-$short; echo "git -C $(squote "$REPO") worktree add --detach $(squote "$SRC") $short"; }
        while IFS=$'\t' read -r l a i; do
            west_words "$l" "$a"
            echo "west $(squote "${WORDS[@]}")   # in ${ws:-<ZEPHYR_WS>}; keep $OUT/$l/zephyr.bin(.sha256), rm -rf $OUT/build-$l"
        done <<< "$SEL_ROWS"
        return 0
    fi
    [ -n "$ws" ] && [ -d "$ws/zephyr" ] || die "set ZEPHYR_WS to the west workspace (it holds zephyr/)"
    [ -f "${GLMARK2_SRC:-}/src/default-benchmarks.h" ] || die "set GLMARK2_SRC to the glmark2 checkout"
    venv=${WEST_VENV:-${ZEPHYR_VENV:-}}
    [ -n "$venv" ] && [ -d "$venv/bin" ] && PATH=$venv/bin:$PATH
    command -v "$WEST" > /dev/null || die "$WEST not on PATH (set WEST_VENV or ZEPHYR_VENV)"
    export ZEPHYR_BASE=$ws/zephyr
    [ -n "${ZEPHYR_SDK_INSTALL_DIR:-}" ] && export ZEPHYR_SDK_INSTALL_DIR
    mkdir -p "$OUT" || die "cannot create $OUT"
    OUT=$(cd "$OUT" && pwd)
    [[ $OUT =~ [:\;[:space:]] ]] && die "out-dir '$OUT': no ':', ';' or blanks"
    if [ -e "$OUT/commit" ] && [ "$(head -n1 "$OUT/commit")" != "$sha" ]; then
        die "refused: $OUT holds images of $(head -c 8 "$OUT/commit"), not $short: use another out-dir" 3
    fi
    echo "$sha" > "$OUT/commit"
    MODS=$("$MODULES_SH" "$ws") || die "zephyr-modules.sh failed"
    if [ "$SRC" != "$REPO" ]; then
        tree=$OUT/tree-$short
        SRC=$tree
        [ -e "$tree" ] || git -C "$REPO" worktree add --detach -q "$tree" "$sha" || die "cannot check out $short"
    fi
    say "build $sha: $(grep -c . <<< "$SEL_ROWS") image(s) into $OUT"
    while IFS=$'\t' read -r l a i; do
        if [ -s "$OUT/$l/zephyr.bin" ] && [ -s "$OUT/$l/zephyr.bin.sha256" ] &&
            (cd "$OUT/$l" && sha256sum -c --status zephyr.bin.sha256); then
            echo "$l sha256=$(cut -d' ' -f1 "$OUT/$l/zephyr.bin.sha256") commit=$short (kept)"
            continue
        fi
        b=$OUT/build-$l
        rm -rf "$b" "$OUT/$l"
        west_words "$l" "$a"
        if ! (cd "$ws" && "$WEST" "${WORDS[@]}") > "$OUT/$l.log" 2>&1 < /dev/null; then
            echo "$l rc=failed (see $OUT/$l.log)"; rm -rf "$b"; rc=4; continue
        fi
        want="CONFIG_GLMARK2_ARGS=\"$a\""
        cfg=$b/zephyr/.config
        got=$(grep -a '^CONFIG_GLMARK2_ARGS=' "$cfg" 2> /dev/null | tail -1)
        if [ ! -s "$b/zephyr/zephyr.bin" ]; then
            echo "$l rc=failed (no zephyr.bin; see $OUT/$l.log)"; rm -rf "$b"; rc=4; continue
        elif [ "$got" != "$want" ]; then
            echo "$l rc=failed (.config has '$got', not '$want')" | tee -a "$OUT/$l.log"; rm -rf "$b"; rc=4; continue
        elif ! grep -aqF "$MARKER" "$b/zephyr/zephyr.bin"; then
            echo "$l rc=refused (no watchdog marker '$MARKER')" | tee -a "$OUT/$l.log"; rm -rf "$b"; rc=4; continue
        fi
        mkdir -p "$OUT/$l"
        mv "$b/zephyr/zephyr.bin" "$OUT/$l/zephyr.bin"
        (cd "$OUT/$l" && sha256sum zephyr.bin > zephyr.bin.sha256)
        rm -rf "$b" "$OUT/$l.log"
        echo "$l sha256=$(cut -d' ' -f1 "$OUT/$l/zephyr.bin.sha256") commit=$short"
    done <<< "$SEL_ROWS"
    if [ -n "$tree" ]; then git -C "$REPO" worktree remove --force "$tree" || say "warning: worktree $tree not removed"; fi
    return $rc
}

# ---- run --------------------------------------------------------------------------
# scrub FILE...: this host's paths in place: <record>, <img-dir>, <repo>, ~
scrub() {
    local f
    for f in "$@"; do
        [ -f "$f" ] || continue
        REC_P=$REC IMG_P=$IMGD REPO_P=$REPO HOME_P=${HOME:-/nonexistent} perl -pi -e '
            s/\Q$ENV{REC_P}\E/<record>/g; s/\Q$ENV{IMG_P}\E/<img-dir>/g;
            s/\Q$ENV{REPO_P}\E/<repo>/g; s/\Q$ENV{HOME_P}\E/~/g' "$f"
    done
}

# picture_line CONSOLE INFO: the scene's glmark2-picture line of a capture
# (console.txt "HH:MM:SS line" or console.raw), without the time and CRs
picture_line() {
    tr -d '\r' < "$1" | sed -E 's/^[0-9]{2}:[0-9]{2}:[0-9]{2} //' |
        grep -aF "glmark2-picture: $2: " | tail -1
}

cmd_run() {
    [ $# -ge 2 ] || die "usage: x2-glmark2-pictures.sh [--dry-run] run <img-dir> <record-dir> [SCENE...]"
    IMGD=$1
    local rdir=$2 sha l a i nn slug img isha8 manifest agg sha8 base stop=0 rc=0
    shift 2
    select_rows "$@"
    [ -d "$IMGD" ] || die "no image directory $IMGD"
    IMGD=$(cd "$IMGD" && pwd)
    sha=$(head -n1 "$IMGD/commit" 2> /dev/null)
    [[ $sha =~ ^[0-9a-f]{40}$ ]] || die "refused: $IMGD/commit holds no 40-hex commit (build the images with build)" 3
    git -C "$REPO" cat-file -e "$sha^{commit}" 2> /dev/null || die "refused: commit $sha is not in $REPO" 3
    on_main "$sha"
    # the selected images, each against its sha256
    while IFS=$'\t' read -r l a i; do
        [ -s "$IMGD/$l/zephyr.bin" ] || die "refused: no image $IMGD/$l/zephyr.bin" 3
        (cd "$IMGD/$l" && sha256sum -c --status zephyr.bin.sha256 2> /dev/null) ||
            die "refused: $IMGD/$l/zephyr.bin does not match its zephyr.bin.sha256" 3
    done <<< "$SEL_ROWS"
    # the manifest of every image in IMGD, in the column's order: the record's name
    manifest=$(while IFS=$'\t' read -r l a i; do
        [ -s "$IMGD/$l/zephyr.bin.sha256" ] && printf '%s  %s\n' "$(cut -d' ' -f1 "$IMGD/$l/zephyr.bin.sha256")" "$l"
    done <<< "$ROWS")
    agg=$(printf '%s\n' "$manifest" | sha256sum | cut -d' ' -f1)
    sha8=${agg:0:8}
    base=$(basename "$rdir")
    if [[ $base =~ ^[0-9]{8}-[0-9]{4}-eye-[0-9a-f]{8}$ ]]; then
        REC=$rdir
        [ "${base: -8}" = "$sha8" ] || die "refused: $rdir is the record of other images (sha8 ${base: -8}, these $sha8)" 3
    else
        REC=$rdir/$(date +%Y%m%d-%H%M)-eye-$sha8
    fi
    if [ "$DRY" = 1 ]; then
        echo "DRY record $REC   # not created: meta.txt (board=x2, board_measured=Armadillo-X2 ..., commit=${sha:0:8}..., image_sha256=$sha8...), images.sha256, console.txt, scenes.txt, photos/, runs/"
    else
        [ -n "${BOARD_UART:-}" ] || die "set BOARD_UART (the X2 console): its 'board: holding' line times the screenshot and its glmark2-picture lines are the record"
        if [ -e "$REC/meta.txt" ]; then
            grep -qx "commit=$sha" "$REC/meta.txt" && grep -qx "image_sha256=$agg" "$REC/meta.txt" ||
                die "refused: $REC/meta.txt names another commit or other images" 3
        else
            mkdir -p "$REC/photos" "$REC/runs" || die "cannot create $REC"
            printf '%s\n' "$manifest" > "$REC/images.sha256"
            {
                echo "board=x2"
                echo "board_measured=$MEASURED"
                echo "commit=$sha"
                echo "route=x2-kexec-dev, hold cue 'board: holding' on the console"
                printf '%s\n' "$manifest" | awk '{ print "image_sha256_" $2 "=" $1 }'
                echo "image_sha256=$agg"
            } > "$REC/meta.txt"
        fi
        mkdir -p "$REC/photos" "$REC/runs"
        REC=$(cd "$REC" && pwd)
        say "record $REC"
    fi
    while IFS=$'\t' read -r l a i; do
        nn=${l:2:2}
        slug=${l:5}
        img=$IMGD/$l/zephyr.bin
        isha8=$(head -c 8 "$IMGD/$l/zephyr.bin.sha256")
        echo "== $nn $l ($i)"
        local -a envc=(env -u NETBOOT_NO_WATCHDOG -u SEL X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev)
        if [ "$DRY" = 1 ]; then
            echo "X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev tools/board.sh --dry-run x2 run $img $REC/runs $l   # BOARD_HOST, BOARD_UART from the environment"
            "${envc[@]}" "$BOARD_SH" --dry-run x2 run "$img" "$REC/runs" "$l" < /dev/null
            if [ -n "${X2_SHOT_CMD:-}" ]; then
                echo "on 'board: holding': $X2_SHOT_CMD $REC/photos/$nn-$slug.png"
            else
                echo "on 'board: holding': shot-now $nn $slug, then a wait of ${X2_SHOT_WAIT:-15} s"
            fi
            continue
        fi
        run_scene "$l" "$i" "$nn" "$slug" "$img" "$isha8"
        case $SCENE_RC in
        0 | 4) ;;
        *) stop=$SCENE_RC ;;
        esac
        [ "$SCENE_PIC" = yes ] || rc=4
        [ "$stop" = 0 ] || break
    done <<< "$SEL_ROWS"
    [ "$DRY" = 1 ] && return 0
    python3 -I "$RECORD_CHECK" "$REC" | tail -1 | sed "s|$REC|<record>|"
    case $stop in
    0) return $rc ;;
    3) say "a run was refused: stopped (nothing was retried)" >&2; return 3 ;;
    5 | 6) say "the board did not come back (rc $stop): stopped, no retry; it needs a person" >&2; return 5 ;;
    *) say "board.sh failed (rc $stop): stopped, no retry" >&2; return "$stop" ;;
    esac
}

# run_scene LABEL INFO NN SLUG IMAGE IMAGE-SHA8: one board.sh run, the capture
# watched for the hold; SCENE_RC and SCENE_PIC set
SCENE_RC=0 SCENE_PIC=no
run_scene() {
    local l=$1 info=$2 nn=$3 slug=$4 img=$5 isha8=$6
    local rcf=$REC/runs/.$l.rc out=$REC/runs/$l.out before rd= cons hold=no shot=none photo line srs
    before=$(ls -d "$REC/runs/"*-"$l-$isha8" 2> /dev/null)
    rm -f "$rcf"
    (
        env -u NETBOOT_NO_WATCHDOG -u SEL X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev \
            "$BOARD_SH" x2 run "$img" "$REC/runs" "$l" < /dev/null
        echo $? > "$rcf"
    ) 2>&1 | tee "$out" &
    local pipe=$!
    photo=$REC/photos/$nn-$slug.png
    while :; do
        if [ -z "$rd" ]; then
            rd=$(ls -d "$REC/runs/"*-"$l-$isha8" 2> /dev/null | grep -vxF -- "$before" | tail -1)
        fi
        if [ -n "$rd" ] && [ "$hold" = no ]; then
            for cons in "$rd/console.txt" "$rd/console.raw"; do
                if [ -f "$cons" ] && grep -aq "$HOLD_RE" "$cons"; then hold=yes; break; fi
            done
            if [ "$hold" = yes ]; then
                if [ -n "${X2_SHOT_CMD:-}" ]; then
                    # shellcheck disable=SC2086  # X2_SHOT_CMD is words
                    if $X2_SHOT_CMD "$photo" < /dev/null && [ -s "$photo" ]; then shot=ok; else shot=failed; fi
                    say "shot $nn $slug: $shot"
                else
                    echo "shot-now $nn $slug"
                    sleep "${X2_SHOT_WAIT:-15}"
                    if [ -s "$photo" ]; then shot=manual; else shot=missing; fi
                fi
            fi
        fi
        [ -s "$rcf" ] && break
        sleep "$POLL"
    done
    wait "$pipe"
    SCENE_RC=$(cat "$rcf")
    rm -f "$rcf"
    [ -n "$rd" ] || rd=$(ls -d "$REC/runs/"*-"$l-$isha8" 2> /dev/null | grep -vxF -- "$before" | tail -1)
    line=
    if [ -n "$rd" ]; then
        if [ "$hold" = no ]; then
            for cons in "$rd/console.txt" "$rd/console.raw"; do
                [ -f "$cons" ] && grep -aq "$HOLD_RE" "$cons" && hold=late
            done
        fi
        if [ -f "$rd/meta.txt" ] && ! { grep -qx 'board=x2' "$rd/meta.txt" && grep -q '^board_measured=Armadillo-X2' "$rd/meta.txt"; }; then
            say "warning: $l: the run's meta.txt lacks board=x2 / board_measured=Armadillo-X2" >&2
        fi
        for cons in "$rd/console.txt" "$rd/console.raw"; do
            [ -f "$cons" ] && line=$(picture_line "$cons" "$info") && [ -n "$line" ] && break
        done
    fi
    srs=$REC/console.txt
    touch "$srs"
    grep -vF "glmark2-picture: $info: " "$srs" > "$srs.new"
    [ -n "$line" ] && echo "$line" >> "$srs.new"
    mv "$srs.new" "$srs"
    SCENE_PIC=no
    [ -n "$line" ] && SCENE_PIC=yes
    echo "$nn $l rc=$SCENE_RC holding=$hold shot=$shot picture=$SCENE_PIC" | tee -a "$REC/scenes.txt"
    scrub "$out"
}

case $VERB in
list) cmd_list "$@" ;;
build) cmd_build "$@" ;;
run) cmd_run "$@" ;;
*) die "unknown verb '$VERB' (list, build, run)" ;;
esac
