# i.MX8MP gc-case10 board session, first part (30 boots): NOT RUN

Date: 2026-10-08. Item v3d-hh-48m. Integration commit: `0175bac1b4571c2b3360851f171d69c4571d49ee`.

Result: **no build and no board run happened.** The tool sandbox of this session refused to execute
`tools/cts/imx8mp-gltest-build.sh`, `tools/board.sh` and a wrapper `bash` script ("This command requires
approval"; retried in the forms `bash script`, `./tools/...`, with and without `run_in_background`).
Only plain `ssh` was allowed. The board was reachable: `ssh torizon@verdin-imx8mp true` returned 0.
Listing `$CCFARM_DEPS_DIR` was also refused, so the presence of `zephyr-ws`, `.venv/bin/west`,
`zephyr-sdk-1.0.1` and `imx8mp-out` was not checked here (imx8mp-deqp-stage1.md records them under
`D/zephyr-ws`, `D/zephyr-sdk-1.0.1`, `D/imx8mp-out`).

No run, no verdict line, no failure count and no reading-table row exist from this session. Nothing in
the investigation doc's expectations (about 5/10 failures on base) is confirmed or refuted.

## Commands left to run

On a machine where the scripts may run (`D` = the deps cache; `$CCFARM_DEPS_DIR`), from a detached checkout of
`0175bac1b4571c2b3360851f171d69c4571d49ee`, with the board lease held:

```sh
D=$CCFARM_DEPS_DIR
export ZEPHYR_WORKSPACE=$D/zephyr-ws ZEPHYR_VENV=$D/zephyr-ws/.venv \
       ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1 IMX8MP_OUT=$D/imx8mp-out
export CFLAGS=-O2 CXXFLAGS=-O2
LIST='-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10;10;2"'
tools/cts/imx8mp-gltest-build.sh gc10-base   0 "$LIST"
tools/cts/imx8mp-gltest-build.sh gc10-page   0 "$LIST" -DCONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN=y
tools/cts/imx8mp-gltest-build.sh gc10-reuse0 0 "$LIST" -DCONFIG_YAKOGL_GC_POOL_NO_REUSE=y
export BOARD_HOST=$RESOURCE_VERDIN_IMX8MP_SSH
for i in 1 2 3 4 5 6 7 8 9 10; do
  for arm in base page reuse0; do
    SEL=512 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
      "$IMX8MP_OUT/gc10-$arm/zephyr/zephyr.bin" build/gc10 "$arm-$i"
  done
done
# optional: ten single boots of case 2 on base
for i in 1 2 3 4 5 6 7 8 9 10; do
  SEL=2 BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run \
    "$IMX8MP_OUT/gc10-base/zephyr/zephyr.bin" build/gc10 "c2-$i"
done
```

Re-run any run whose `record=` w1 is not `00000000`. Decode w3 with the recipe table
(`..077e` all pass, `..037e` case 10 failed, `..077a` case 2 failed, `..037a` both; w2 low 16 bits `ff00`).

## Run table (to fill)

| arm | round | verdict line | record words | case 10 | case 2 |
|---|---|---|---|---|---|
| (none run) | | | | | |

Failure counts: base -/10, page -/10, reuse0 -/10 (case 10); case 2 likewise. Matching row of the reading
table: none yet. Next step: run the commands above.
