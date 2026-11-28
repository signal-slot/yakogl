# tools/pvr-hwperf

Host decoder for the console lines `pvr_fw_hwperf_drain` prints
(`driver/src/pvr_fw_boot.c:450`, which calls `hwperf::drain_log`).

    python3 tools/pvr-hwperf/decode.py [--core-hz 800000000] [console.log]
    sh tools/pvr-hwperf/test_decode.sh      # self-test on fixture.log

Standard library only; stdin when no file is given. The core clock defaults
to the driver's `PVR_CORE_CLOCK_HZ` (`driver/src/pvr_fw_boot.c:283`).

## Log line format

Each drain is a totals line, then one line per lane
(`driver/rust/src/gpu/pvr/hwperf.rs:831` `drain_log`; any log prefix before
`pvr-hwperf` is ignored, fields are `name=value`):

- `pvr-hwperf: from= widx= wrap= size= fw_drops= packets= dropped= other=
  tasks= unmatched= open= overflow= span_ticks= span_ns= err=`
  (`hwperf.rs:841`)
- `pvr-hwperf lane=<ta|3d|3dspm|3dtq|cdm|tla|tdm> tasks= gpu_ticks= gpu_ns=
  k2s_n= k2s_ticks= k2s_ns=` (`hwperf.rs:867`; lane names from `Lane::name`,
  `hwperf.rs:514`)

Per drain the table gives per lane: count, total, mean and max GPU time and
the kick-to-start mean (k2s_ticks / k2s_n), all in microseconds
(ticks / core_hz).

## Caveats

- The lane line has no per-task maximum: `max_us` is `-` unless job lines
  are present.
- Job lines are **provisional**: the per-job-ref format is not on main yet
  (`ext_job_ref` is read at `hwperf.rs:243` but `drain_log` does not print
  it). The decoder accepts `pvr-hwperf job ext=<ref> lane=<lane>
  gpu_ticks=<n> k2s_ticks=<n>` after a drain's lane lines, gives a max from
  them and a per-job-ref table. If pvr-hwperf-jobref prints another format,
  adjust `parse()` in `decode.py` and `fixture.log` together.
