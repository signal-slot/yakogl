# stage 2 benchmark gap audit, 2026-10-15, main 548860df

## State of main
No change to main's behaviour. `docs/investigations/stage2-bench-gap.md` is
new: the board x benchmark table for stage 2. Finding: glmark2 pictures are
not checked per scene today (no read-back or reference in
`driver/samples/zephyr/glmark2/src/main.cpp`; `tools/cts/common-tally.py`
`glmark2_scenes()` judges console error lines only).

## Open work
Ten numbered items that close stage 2 are at the end of
`docs/investigations/stage2-bench-gap.md`: per-scene picture value in the
image and tally, i.MX8MP per-scene evidence (no console), fps for Slint and
for Pi 4 / i.MX8MP, AM62P at 256 MiB (plan #149 am62p-common-carveout),
AM62P SV repeat, terrain and refract on a board, Slint Pi 4 fingerprint
decision, cluster "same picture", one-commit README table.

## Known defects
- AM62P SV returned 0 console bytes at cd738b1a (not repeated).
- Slint picture differs on the Pi 4 by one RGB565 step (crc16 312b vs c592).
- Cluster crc16 values are not comparable across the three panel sizes.

## Next board session
AM62P: the common set at a commit containing 504d4fae, SV repeated. Pi 4 and
i.MX8MP: nothing until the picture check of item 1 exists.
