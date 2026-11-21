# Pi 4: reading the `glmark2-jobs` line (fields, the `~` mark, the 23.49 to 27.26 ms render sum)

Plan #329 (#1209), item `v3d-jobs-readout`. A code reading; no board run, no
code change. The records of the board runs are gitignored and not on the
machine that wrote this, so every board figure below is quoted from
`docs/notes/` (with its place), none re-read from a record. Code lines are
of `src/osal/zephyr_v3d.c` (OSAL) and
`driver/samples/zephyr/glmark2/src/main.cpp` (`main.cpp`) at the tree this
note was written on (`93eb0f0a`).

## Answer in short

- `render` is **not** the GPU's render time. It is "render start (ISR at
  FLDONE) to the moment the OSAL *looked* and saw FRDONE". When the CPU
  looks late, the late time is in `render`, and `~` marks exactly that
  case. A `~` entry is an upper bound on the render, by an amount the line
  does not give.
- `idle` does **not** contain that lateness (it is measured from the OSAL's
  look, not from the GPU's FRDONE), so a sum of `idle` under-reports the
  time the GPU really stood idle. The real exposed gap of a `~` job is
  (`render` - true render) + `idle`; the true render is not printed.
- The 3.77 ms rise of frame 10's render sum (23.49 to 27.26 ms) cannot be
  split into "GPU got slower" and "OSAL looked later" from the numbers in
  the tree. What the code allows is the mechanism (a late look) and a size
  bound (below). The reconciliation needs the per-job entries of the old
  records, which are not in the tree.
- Candidates 1 and 2 of `pi4-terrain-cpu-left.md` are read off the line
  differently from what that note says: see the end.

## 1. The fields, each with its start and end event

One ring entry is written per job by `note_job_done` (`zephyr_v3d.c:759-784`),
read by `osal_v3d_recent_jobs_us` (`:786-806`), converted from cycles to us
(`k_cyc_to_us_floor32`, floor), and printed by `print_jobs`
(`main.cpp:1402-1421`). The ring carries `fence`, `prep`, `bin`, `render`,
`clean`, `total`, `render_late`, `idle`, `tfu_n`, `tfu_cyc`
(`struct recent_job_cyc`, `:746-751`); **`print_jobs` prints only `total`,
`render`, `~`, `idle` and `tfu`** (`main.cpp:1414-1417`). `prep`, `bin` and
`clean` are in the ring and are not on the line.

| printed | start event | end event | code |
|---|---|---|---|
| `total` | `c0`: the job's `z_submit` entry (`const uint32_t c0 = k_cycle_get_32()`, `:1398`, after the pending job is finished at `:1396`) | `c_end`: after the render was seen, the cache clean, and the TFU jobs queued behind this job (`:1311-1314`) | `last_job_cyc.total = c_end - s.if_c0` (`:1314`), `s.if_c0 = c0` (`start_job`, `:1442`) |
| `render` | `s.if_c_render`: taken in the ISR at FLDONE, just before `v3d_submit_render` (`:1047`) | `c_frdone`: **the moment `finish_inflight` returned from `v3d_wait` on FRDONE** (`:1281`), `last_job_cyc.render = c_frdone - s.if_c_render` (`:1283`) | the header comment of `finish_inflight` says it: "the render's start to the moment this function saw FRDONE, so it includes any time the CPU spent elsewhere before asking" (`:1242-1252`) |
| `~` | - | `render_late = 1` when `INT_STS.FRDONE` was **already set** when `finish_inflight` first read the register, before it waits (`:1275-1276`); reset to 0 in `start_job` (`:1428`) | comment `:1273-1274`: "FRDONE has no interrupt: when it is already up, the render ended at an unknown time before now, and the end stamp below is this look's" |
| `idle` | `recent_prev_end`: the OSAL's stamp when the previous job's last GPU work ended: `c_end` of the previous job (`note_job_done`, `:783`) or the end of a TFU job (`tfu_now`, `:1360`), whichever came last | `s.if_c_bin`: this job's bin submit (`start_job`, `:1443`) | `gap = if_c_bin - recent_prev_end`; `idle = (recent_n == 0 \|\| gap < 0) ? 0 : gap` (`:769`, `:778`). First job since init: 0; crossed stamps: 0 |
| `tfu K/Tus` | each TFU job's `v3d_tfu_run` start (`tfu_now`, `:1357`) | its end (`:1360-1361`) | `pend_tfu_n`, `pend_tfu_cyc` are summed since the previous `note_job_done`, copied into this job's entry and zeroed (`:779-781`). Printed only when K != 0 (`main.cpp:1416`) |

Not on the line but in the ring: `prep = c_bin - c0` (`:1434`: this includes
the **wait behind the job in flight**, see 3, plus the tile-state `memset`
of `start_job`, `:1431`); `bin = if_c_fldone - if_c_bin` (`:1269`, from the
FLDONE ISR's stamp `:1034`); `clean = (end of v3d_caches_clean) - c_frdone`
(`:1302-1303`).

Which jobs are listed: those whose swap-time count changed in swap number 10
(`JOBS_LINE_FRAME`, `main.cpp:195`): `n = j.jobs - done_before`, the last `n`
ring entries (`main.cpp:628-640`). The text above `print_jobs`
(`main.cpp:1399-1400`) says a frame's last render "finishes at the next
submit", so the list can carry the previous frame's last job; for terrain
`pi4-slow-terrain.md` records that every swap is waited for, so mostly it
does not, but the first entry is the one at risk of being the previous
frame's (not checked on a record).

## 2. What `~` means

The render's end is not interrupted: the OSAL reads FRDONE only when
something asks (`wait`, `completed()`, the next `z_submit` with a job
waiting, an allocation, an osal_v3d_tfu: `finish_inflight` callers,
`:1366`-`:1495`). `~` says: at the first look the bit was up already, so the
GPU had finished *before* the look, by an unknown time, and `render` is
(true render) + (that unknown lag). Without `~`, the look found FRDONE
down and `v3d_wait` returned at the interrupt-free poll's next read, so
`render` is the true render plus at most a poll step.

Note what `~` does *not* say: it does not say the *job* was waited for; it
says the *look* was late. A job finished by `z_submit` of the next job
(`:1396`, `finish_inflight(false)`) is the typical late case: the lag is the
CPU time the application spent recording since its last look.

## 3. Can an entry span two jobs, or include TFU or wait time?

Findings, from the code:

- **Two jobs: no.** One entry per `note_job_done`, one per fence. But the
  *printed list* of a swap can mix frames (end of section 1).
- **`render` includes CPU time before the look** (late look), by design
  (`:1242-1252`). This is the one place wait or CPU time enters it. A TFU
  job does not enter `render`: `c_frdone` is taken before the TFU jobs run
  (`:1281` against `:1311`). The queued TFU time is in `total` and in the
  entry's `tfu`, not in `render`.
- **`total` includes**, in order: `prep` (the time behind the job in flight,
  `:1398` takes `c0` at submit; a job that has to wait for the
  running one starts its bin only at `start_pending`, so its `total` counts
  from submit, not from the start of its GPU work), the bin, the render
  (with any lag), the clean, and the TFU jobs queued behind it
  (`tfu_queue_run`, `:1111-1139`, run at `:1311` before `c_end`).
- **`tfu` can lie outside `total`.** A TFU run through `osal_v3d_tfu`
  directly (the GPU idle: `OSAL_QUERY_V3D_TFU_RUN`, `:1586-1596`, and the
  `_QUEUE` branch with no job in flight, `:1622-1626`) runs after
  `finish_all()` (`:1344`), i.e. **after** the job it follows has been
  noted. Its time goes to `pend_tfu_*` and comes out in the **next** job's
  entry, whose `total` started at that job's `z_submit`, possibly before or
  after that TFU. So `total - render - tfu` may subtract a TFU that `total`
  never counted (the result can then be too small, even negative in
  principle), and queued TFU that `total` did count are in `tfu`.
- **`idle` is the OSAL's idle, not the GPU's.** It starts at the OSAL's
  `c_end` of the previous job, not at that job's FRDONE. The lag of a `~`
  job (section 2) is inside that job's `render`, so the GPU's real idle
  after a `~` job = lag + `idle`; the line shows only the `idle` part.
  A job started by `start_pending` right after its predecessor was noted
  has `idle` of a few us by construction (`:1477`, the stamp is taken in
  `note_job_done` just before `start_pending`).

### The 23.49 to 27.26 ms change

- The two figures are the sum of `render` of the frame-10 entries on the
  same line, same code (`print_jobs` and `render_late` have not changed
  since the 0.1.0 import: `git log -S render_late` shows only the import;
  `zephyr_v3d.c` itself was last touched by the present-op commit
  `7a20fcb5`). So **yes, the earlier 23.49 ms is the same field**:
  `pi4-slow-terrain.md:442` quotes it for both `c707598d` and `17e91019`
  ("8 jobs, 23.49 ms render"). The lines themselves are only in
  gitignored records; `pi4-glmark2-cpu.md:556` quotes only the sum of
  work (29.1 ms = render 23.49 + TFU 1.83 at that time per the table at
  `pi4-slow-terrain.md:442`; the arithmetic of 29.1 against 8 jobs is the
  same note's, not re-derived) and `v3d-terrain-gpu.md` (`:44-52`) says the
  per-job times are not in the tree. `pi4-terrain-cpu-left.md:33` and
  `:204-208` use 23.49 as "measured (`glmark2-jobs`, frame 10)".
- Two figures identical to 0.01 ms in two different images (`pi4-slow-terrain.md:442`,
  before `c707598d` and after `17e91019`) and a different 27.26 at
  `e07d066a` is itself information: with 9 jobs, one of them the dead
  window clear, the renders were stable; at `e07d066a` that job is gone
  (9 jobs to 8, `pi4-slow-terrain.md:575-580`) and the *remaining* sum is
  3.77 ms **higher**. Removing a job cannot add GPU render work. What the
  code allows to move is therefore (a) a different job now being the one
  that is looked at late (the dead clear's removal changes which `z_submit`
  or wait finishes which job), or (b) one of the render times truly
  longer (e.g. the first job after a TFU, or a different cache state).
- The mark `4394~` fits (a) in size: one entry printed `4394~` is a `~`
  entry, whose `render` is at least its true render; 3.77 ms of the 4.39 ms
  is within it if that job's true render is as short as ~0.6 ms (a short
  window/blit pass; the other jobs of the frame, per
  `v3d-terrain-gpu.md`, are 7.2 ms and 2 x about 1 ms for the 800x600
  passes). This is an **inference about size, not a measurement**: which
  job `4394~` is, and what the old line printed for it, is not in the tree.
- **Cannot be reconciled from the code**: the verdict needs (i) the
  `glmark2-jobs` line of `e07d066a`'s frame 10 (all entries, with `~`) and
  (ii) that of the `17e91019` image. Both are in the gitignored records of
  the board runs. Then: if only `~` entries grew, it is lag; if a non-`~`
  entry grew, it is the GPU.

A second clue worth keeping: the sum of the old and new lines should be
compared on the **non-`~` entries only**, plus `total - tfu - bin - prep -
clean` for the `~` ones if `prep`, `bin` and `clean` are printed (see the
next section's last item).

## 4. Reading candidates 1 and 2 of `pi4-terrain-cpu-left.md`

That note (`:237-270`) reads candidate 1 (overlap the next bin with the
render) from "`total - render - tfu` summed over the 8 jobs: 1 to 5 ms if bin
is the item" and candidate 2 (start the pending job from the CPU without a
wait) from "the sum of `idle`", with the gain "at most that sum". Corrected:

1. **`total - render - tfu` is not the bin.** It is `prep + bin + clean`
   plus the glue between them (and a possible TFU mismatch, section 3). In
   particular `prep` contains the **time the job spent submitted but
   waiting behind the running one** (`:1398`, `:1405-1421`, then `start_pending`),
   which is the job's neighbour's GPU time and not a cost of the bin. For
   a frame of jobs submitted back to back, `prep` is mostly overlap with
   the previous job's render. Only `bin` is candidate 1's quantity (it is
   the part overlap could hide), and `bin` is not on the line. The line
   **overstates** candidate 1; do not read it as 1 to 5 ms of bin.
   **Read candidate 1 from `bin`** per job (in the ring,
   `osal_v3d_recent_jobs_us` carries it; `print_jobs` does not print it;
   adding it is a one-line change of `main.cpp:1414`, not made here). As
   the note says "job 3's `bin` above 1 ms confirms"; that sentence needs
   the print.
2. **The sum of `idle` is a lower bound of candidate 2's window**, not
   its size, for two reasons: (a) `idle` starts at the OSAL's end stamp
   (section 3), so the lag of a `~` job is not in it; (b) `idle` is 0 for the
   first job after init and when the stamps cross. The size of candidate 2's
   window is `sum(idle) + sum(render - true render for ~ jobs)`; the second
   term is not printed. A `~` on a job *is* the evidence for candidate 2
   (the GPU had finished and the OSAL had not noticed): **count the `~`
   entries**. `4394~` is the first; the number of others on the line is
   the first thing to read.
3. A cheap readout that needs no code: for non-`~` jobs `render` is a true
   figure and `total - render - tfu` is prep + bin + clean (upper bound of
   bin); for `~` jobs treat `render` as an upper bound only. For a clean
   candidate-1 readout the print should add `bin` and `prep` (and `clean`)
   per job; that and a `render` lag counter are the two additions the
   planner may want as an item (touching `main.cpp` and the OSAL, which
   this item may not).

## Limits

- No board and no record read; every board number is quoted from the
  notes cited. The line-number citations of this note were read on the tree
  above; the `~`-sized inference in section 3 is not a measurement.
- Whether the first entry is the previous frame's last job was not
  checked.
