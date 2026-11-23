# PowerVR HWPerf: how the firmware's own timestamps would split a kick's ~0.9 ms

Study, no code. Plan #329, #1199. `am62p-slow-terrain.md` ('What a mip-level
kick pays whatever its size', 'Open') finds that a queued mip kick costs about
1.1 ms, of which the GPU is busy about 0.19 ms and the host about 5 us; about
0.9 ms lies between the post and the geometry fence, inside the firmware.
`pvr-fence-wait.md` (lines 226, 333, 432, 661-662) says only the firmware's
HWPerf timestamps can split it. This note says how to turn them on in our
firmware init, which events to ask for, how to read them without an
interrupt, what a gl_test case 216 print would look like, and what each
reading of the 0.9 ms predicts.

Sources: the vendored DDK 25.2 headers under `third_party/ti-img-rogue-ddk-25.2`
(`UPSTREAM`: rogueddk 25.2@6850647) and our firmware init in
`driver/rust/src/gpu/pvr/`. Paths below are relative to the DDK directory
when they start with `include/`, `hwdefs/` or `config/`. A claim marked
**[knowledge]** comes from knowing the DDK's `.c` files (`rgxhwperf.c`,
`rgxfwutils.c`, `rgxtimecorr.c`), which are not in the tree (only
`services/server/devices/rogue/rgxstartstop.c` is vendored); the first board
session has to check it.

## Today

- `fwinit.rs:246-255`: a zeroed `RGXFWIF_HWPERFDATA` (40 bytes,
  `fwif.rs:174`) goes to `OSINIT.sHwPerfData` (`fwif.rs:219`). The comment
  says a filter of 0 keeps the firmware from emitting its boot packet,
  "which would fault on the unset L1 buffer": so this firmware build has the
  HWPerf producer in it and reads the filter at boot.
- `fwinit.rs:257-261`: a zeroed 512-byte `RGXFWIF_RUNTIME_HWPERFCFG` goes to
  `OSINIT.sHwPerfRuntimeCfg` (`fwif.rs:220`).
- `fwinit.rs:292-296`: a zeroed 2048-byte HWPerf counter-block control goes
  to `SYSINIT.sHWPerfCtl` (`fwif.rs:228`). That is for hardware counter
  blocks, not for the events this note needs; it stays zeroed.
- `fwinit.rs:180-183`, `:198`: `RGXFWIF_SYSDATA.ui32ConfigFlags` is
  `INICFG_CTXSWITCH_PROFILE_MEDIUM` (plus `ASSERT_ON_HWR` on a knob), no
  `INICFG_HWPERF_EN`.
- `fwinit.rs:145`, `:221-222`, `:255`: `RGXFWIF_GPU_UTIL_FW`, which holds the
  timer correlation array, is allocated and left zeroed.

## (a) What the firmware needs, and where each goes in our init

| item | DDK definition | value | where in `fwinit.rs::build` |
|---|---|---|---|
| global HWPerf enable | `RGXFWIF_INICFG_HWPERF_EN` = bit 2 (`include/rogue/rgx_fwif_km.h:147`), a flag of `RGXFWIF_SYSDATA.ui32ConfigFlags` (`include/rogue/rgx_fwif_shared.h:676`) | `sys_flags \|= INICFG_HWPERF_EN` | beside `:180-183`, before the write at `:198` |
| event filter | `RGXFWIF_HWPERFDATA.ui64HWPerfFilter` (`include/rogue/rgx_fwif_km.h:1687`), a 64-bit mask of `RGX_HWPERF_EVENT_MASK_VALUE(type)` = `1 << type` (`include/rogue/rgx_hwperf.h:347`) | the mask of (b) | in the object `obj` allocates at `:249`, offset 32 |
| buffer size | `RGXFWIF_HWPERFDATA.ui32HWPerfSize` (`include/rogue/rgx_fwif_km.h:1675`, "Constant after setup, needed in FW") | the buffer's bytes | same object, offset 12 |
| buffer | `RGXFWIF_RUNTIME_HWPERFCFG.sHWPerfBuf` (`include/rogue/rgx_fwif_km.h:1662-1666`, "On-demand allocated HWPerf buffer address, to be passed to the FW") | firmware address of a new firmware-heap object (`heap_fwaddr`, `mem.rs:108`) | the object at `:259`, offset 0 |
| read/write indices | `sHWPerfCtrl.{ui32HWPerfRIdx, ui32HWPerfWIdx, ui32HWPerfWrapCount}` (`include/rogue/rgx_fwif_km.h:1669-1674`) | 0 | offsets 0, 4, 8 of the `:249` object (already zero) |

The offsets follow from the struct (`include/rogue/rgx_fwif_km.h:1669-1688`:
three `u32` indices, size, drop count, three `u32` utilisation and drop
ordinals, then an 8-aligned `u64` filter at 32), which ends at 40 bytes, the
size `fwif.rs:174` pins. New constants go into `fwif.rs`'s `pins!` table
(`fwif.rs:12-17`, `:19-25`), so the gate's `_Static_assert`s check each
against the headers: `offsetof(RGXFWIF_HWPERFDATA, ui64HWPerfFilter)`,
`..., ui32HWPerfSize)`, `..., sHWPerfCtrl.ui32HWPerfWIdx)`,
`offsetof(RGXFWIF_RUNTIME_HWPERFCFG, sHWPerfBuf)`,
`RGXFWIF_INICFG_HWPERF_EN`.

Size. The DDK's AM62P build uses a 2048 KB firmware buffer
(`config/am62p_linux_release/config_kernel.h:75`,
`PVRSRV_APPHINT_HWPERFFWBUFSIZEINKB 2048`) and a filter of 0 by default
(`:109`, `PVRSRV_APPHINT_HWPERFFWFILTER 0`). Case 216's 20 frames of about 40
kicks with the events of (b) need about 0.6 MB (the packet sizes in (b)):
2 MB holds a whole case without the host ever freeing space. Our firmware
heap is 32 MiB (`fwif.rs:56-57`); the init logs how far it is used
(`fwinit.rs:298`), which says whether 2 MB fits.

Two ways to set the filter, both in the headers:
1. at init, in `ui64HWPerfFilter`, with the buffer address set before the
   firmware starts (the boot packet then has somewhere to go,
   `fwinit.rs:246-248`);
2. at run time, `RGXFWIF_KCCB_CMD_HWPERF_UPDATE_CONFIG`
   (`include/rogue/rgx_fwif_km.h:1002-1004`, "Configure HWPerf events ... and
   HWPerf buffer address (if required)") with `RGXFWIF_HWPERF_CTRL`
   (`:703-706`, `eOpCode` `RGXFWIF_HWPERF_CTRL_SET` `:696`, `ui64Mask`),
   posted with `kccb_post` (`ccb.rs:68`). **[knowledge]** The DDK allocates
   the buffer on demand, writes its address into the runtime config and then
   sends this command; the firmware re-reads `sHWPerfBuf` on it.

Route 1 is the smaller change and needs no new KCCB command; route 2 lets a
gl_test case switch the events on only around its frames (the cost of (f)).
The sketch in (d) uses route 1 and keeps route 2 as the fallback if the
firmware ignores a filter set before boot.

Not to set: `RGXFWIF_INICFG_HWP_DISABLE_FILTER` (bit 14,
`include/rogue/rgx_fwif_km.h:160`); the header does not say what it does.
`SYSINIT.sHWPerfCtl` (`fwinit.rs:292-296`) stays zeroed: no counter blocks.

## (b) Events, packet layout, clock

### The events

Type numbers from `include/rgx_hwperf_common.h`; the mask bit is the type.

| event | type | what it stamps | used for |
|---|---|---|---|
| `RGX_HWPERF_HW_TAKICK` | 0x08 (`:104`) | "TA task started" on the hardware | TA start |
| `RGX_HWPERF_HW_TAFINISHED` | 0x09 (`:105`) | TA finished | TA end |
| `RGX_HWPERF_HW_3DKICK` | 0x0B (`:107`) | 3D started | 3D start |
| `RGX_HWPERF_HW_3DFINISHED` | 0x0C (`:108`) | 3D finished | 3D end |
| `RGX_HWPERF_HW_3DSPMKICK`, `_3DSPMFINISHED` | 0x11, 0x18 (`:113`, `:121`) | partial-render 3D | a partial render in a mip kick would show here |
| `RGX_HWPERF_HW_PMOOM_TAPAUSE`, `_TARESUME` | 0x07, 0x19 (`:102`, `:123-124`) | TA paused for memory | the freelist |
| `RGX_HWPERF_FW_BGSTART`, `_BGEND` | 0x01, 0x02 (`:89-90`) | "Background task processing start/end" | **[knowledge]** the firmware's KCCB and client-CCB processing runs as its background task: the firmware's take-up of a post |
| `RGX_HWPERF_FW_IRQSTART`, `_IRQEND` | 0x03, 0x04 (`:91-93`) | "IRQ task processing" | the firmware's handling of the TA/3D done interrupts |
| `RGX_HWPERF_UFO` | 0x38 (`:161`) | "FW UFO Check / Update" | when the firmware checks a kick's fences (`CHECK_SUCCESS`, `CHECK_FAIL`, `:931-934`) and writes its UFOs (`UPDATE`, `:930`) |
| `RGX_HWPERF_CSW_START`, `_FINISHED` | 0x30, 0x31 (`:154-155`) | context store | context switching is on for every DM (`fwinit.rs:185-195`) |
| `RGX_HWPERF_PWR_CHG`, `_GPU_STATE_CHG`, `_CLKS_CHG` | 0x23, 0x1B, 0x1A (`:141`, `:131`, `:130`) | power state, GPU work state, clock | a power transition between kicks |
| `RGX_HWPERF_FWACT` | 0x39 (`:162`) | firmware activity: `HWR_*`, `FILTER_SET`, `KICK_CANCEL` (`:1094-1105`) | a recovery or a cancelled kick inside the window |

The DDK names ready-made masks for these: `RGX_HWPERF_EVENT_MASK_FW_SED`
(the BG/IRQ/DBG range, `include/rogue/rgx_hwperf.h:417`),
`_FW_UFO` (`:419-420`), `_FW_CSW` (`:421-423`), `_HW_KICKFINISH` (all HW
kick/finish events, `:433-436`), `_ALL_PWR` (`:444-447`) and the DDK's own
default `FWACT | PWR_CHG | CLKS_CHG` (`:408-411`). The filter for the probe:

```
HW_KICKFINISH | FW_SED | FW_UFO | FW_CSW | ALL_PWR | MASK_VALUE(FWACT)
```

The host-side `RGX_HWPERF_HOST_ENQ` (`include/rgx_hwperf_common.h:189-191`,
`include/rogue/rgx_hwperf.h:451-452`) is a packet the DDK's server writes
itself, not the firmware; our post time comes from the kick stamps instead
(`kick_times.rs:33-54`, `t_posted`).

### The packet

Every packet starts with `RGX_HWPERF_V2_PACKET_HDR`
(`include/rgx_hwperf_common.h:295-301`), 24 bytes:

| offset | field |
|---|---|
| 0 | `ui32Sig`: `HWPERF_PACKET_V2B_SIG` 0x48575042 'HWPB' (`:278`; any of the four signatures `:285-289` is valid) |
| 4 | `ui32Size`: whole packet in bytes, low 16 bits (`RGX_HWPERF_SIZE_MASK`, `:308`) |
| 8 | `eTypeId`: event type in bits 0-14 (`RGX_HWPERF_TYPEID_EVENT_MASK`, `:356`), also thread, stream, OSID bits (`:357-362`) |
| 12 | `ui32Ordinal`: packet sequence number (a gap is a dropped packet) |
| 16 | `ui64Timestamp` |

Packets are 8-byte aligned (`RGX_HWPERF_MAKE_SIZE_FIXED`, `:326-329`) and at
most 3 KB (`:315`).

A HW event (TAKICK ... 3DFINISHED) carries `RGX_HWPERF_HW_DATA` (`:633-669`)
after the header, 60 bytes before the optional counter stream: so about 88 bytes
a packet with no counter blocks. The fields that tie a packet to a kick:
`ui32DMContext` (`:646`, the firmware render context), `ui32WorkTarget`
(`:648`, the render target, i.e. the HWRT data), `ui32ExtJobRef` (`:650`) and
`ui32IntJobRef` (`:652`), which **[knowledge]** the firmware copies from the
client-CCB command header's `ui32ExtJobRef`/`ui32IntJobRef`
(`include/rogue/rgx_fwif_km.h:1395-1402`). Our command headers are zeroed
before they are filled (`render.rs:1130`), so today both are 0; the sketch
writes the kick's sequence number there. `ui32DMCyc`/`ui32KickStartTime`
(`:635-641`) and `ui32KickInfo`/`ui32KickEndTime` (`:661-663`) are for
"Pipeline DMs" only.

A UFO packet carries `RGX_HWPERF_UFO_DATA` (`:959-983`): `eEvType`, the job
refs, `eDM`, then `ui32StreamInfo`-encoded tuples of `{ui32FWAddr, value}`
(check success), `{addr, value, required}` (check fail) or
`{addr, old, new}` (update) (`:940-957`). The UFO addresses are the ones our
command words name, so the geometry and fragment fences of each kick can be
matched by address. A FW event (BG/IRQ) carries `RGX_HWPERF_FW_DATA`
(`:609-617`).

Per kick with the filter above: 4 HW packets, about 4 UFO packets (geometry
check and update, fragment check and update) and a few BG/IRQ packets, about
0.7 KB. 20 frames x 40 kicks: about 0.6 MB.

### The clock and its conversion

`include/rogue/rgx_fwif_km.h:1898-1903`: "On the FW the RGX_CR_TIMER counter
is used to keep track of time; it increments by 1 every 256 GPU clock
ticks". `RGX_CR_TIMER` is register 0x0160 with a 48-bit value
(`hwdefs/rogue/km/rgx_cr_defs_km.h:1655-1661`); our driver already reads it
(`driver/src/pvr_render.c:342`, `pvr_bringup.c:232-244`). At our 800 MHz core
clock (gl_test case 216 divides cycle counters by 800 for microseconds and
multiplies `CR_TIMER` deltas by 256, `gl_test.c:20502-20506`) one tick is
0.32 us, fine enough for a 0.9 ms gap.

**[knowledge]** The firmware writes the raw `CR_TIMER` value into
`ui64Timestamp`, and the DDK's host converts it with the
`RGXFWIF_TIME_CORR` entry the packet's `ui32TimeCorrIndex` names
(`include/rogue/rgx_fwif_km.h:1885-1897`: `ui64OSTimeStamp`,
`ui64CRTimeStamp`, `ui64CRDeltaToOSDeltaKNs`; the array of 256 in
`RGXFWIF_GPU_UTIL_FW`, `:1946`, `:1955-1964`;
`RGXFWIF_GET_DELTA_OSTIME_NS`, `:1928-1929`). The host fills that array
(`rgxtimecorr.c`, not vendored); ours is zeroed (`fwinit.rs:221-222`). Rather
than fill it, our reader makes its own correlation: read the CPU counter
(`cntvct_el0`, `kick_times.rs:125-129`) and `CR_TIMER` back to back before
the frames, and again after, and convert with
`t_cpu = t0_cpu + (cr - cr0) * 256 / core_hz` (the slope checked by the
second pair). The first board run checks the [knowledge] part: if a
`TAKICK` timestamp is within a few ms of a `CR_TIMER` read on the host, it is
raw; if it is near 0 or near `cntvct`, the firmware converts and the reader
changes accordingly.

Caveat: `rgxlayer.rs:82` lists the `SOC_TIMER` feature for our core; the
firmware may use the SoC timer for its deadline scheduling (the runtime
config's SOC clock, `fwinit.rs:202-214`). The header says HWPerf uses
`CR_TIMER`; the same first-run check covers it.

## (c) Reading the buffer from the host

No interrupt is needed. **[knowledge]** The firmware writes each packet at
`WIdx`, then advances `WIdx`; when a packet does not fit before the end, it
sets `WrapCount` to where the data ends and goes on at 0; it does not write
past `RIdx` and counts a packet it cannot place in `ui32HWPerfDropCount`
(`include/rogue/rgx_fwif_km.h:1676-1677`). The DDK reads the buffer from its
MISR on any firmware interrupt; polling the same indices is equivalent.

The reader, after the measured frames (or at any point):
1. invalidate the 40-byte `RGXFWIF_HWPERFDATA` (`KM::invd`, `km.rs:331-336`)
   and read `WIdx`, `WrapCount`, `DropCount`;
2. invalidate `[RIdx, WIdx)` of the buffer (two ranges after a wrap);
3. walk the packets: check `ui32Sig` (`HWPERF_PACKET_ISVALID`,
   `include/rgx_hwperf_common.h:285-289`), take `ui32Size & 0xFFFF`, check
   `ui32Ordinal` increments by one;
4. do not write `RIdx` back (see below).

Cache handling on the AM62P. The firmware reaches every host-shared object
through its uncached window when `YAKOGL_DRV_GPU_PVR_FW_UNCACHED` is on, the
default (`driver/zephyr/Kconfig:142-159`, `mem.rs:93-112`): the HWPerf data
and buffer are allocated with the same `obj`/`heap_fwaddr` and get the same
treatment, so the firmware's writes reach memory. Our CPU view of the
firmware heap is cacheable (`ccb.rs:302-306`: "Ours reads through the CPU
cache"), so the reader must invalidate before each read, as the FWCCB
consumer does (`ccb.rs:302-308`). The FWCCB showed the trap of writing back:
`read_offset` (ours) and `write_offset` (the firmware's) share a 64-byte
line and `flush` after our write writes back our stale copy of the
firmware's word (`ccb.rs:307-311`). `RIdx`, `WIdx` and `WrapCount` sit in
one 12-byte struct (`include/rogue/rgx_fwif_km.h:1670-1674`), in one line:
writing `RIdx` with a cache flush would write back a stale `WIdx`. The
sketch avoids it by never writing `RIdx` during a capture: a 2 MB buffer
holds a whole case 216 run (a). A long capture (glmark2) would need a
non-temporal 32-bit store of `RIdx` with no flush of the line, or the
firmware's view through route 2; that is for later.

## (d) Diff sketch

Off by default; with the knob off, the init words and every code path are
what they are today.

- `driver/zephyr/Kconfig`: `config YAKOGL_DRV_GPU_PVR_HWPERF` (bool, default
  n, depends on `YAKOGL_DRV_GPU_PVR`, "the firmware's HWPerf events into a
  buffer the host reads after a measurement; an instrument") and
  `YAKOGL_DRV_GPU_PVR_HWPERF_BUF_KB` (int, default 2048), beside
  `YAKOGL_DRV_GPU_PVR_KICK_TIMES` (`:198-209`). The Rust side as a
  `pvr-hwperf` feature beside `pvr-kick-times` (`driver/rust/Cargo.toml:63`).
- `driver/rust/src/gpu/pvr/fwif.rs`: the pinned constants of (a) and the
  event types and masks of (b).
- `driver/rust/src/gpu/pvr/fwinit.rs`: a flag `FWINIT_HWPERF` beside
  `FWINIT_PARTIAL_RENDER` (`:52-60`) and `hwperf_buf_size` in `YdPvrFwinit`
  (`:33-47`, and its C twin). In `build`: with the flag, `obj` the buffer
  (4 KiB aligned), write its firmware address into the runtime HWPerf
  config (`:259-261`), the size and filter into the HWPerf data
  (`:249-253`), `INICFG_HWPERF_EN` into `sys_flags` (`:180-183`); keep the
  heap offsets of the data and the buffer in `YdPvrKm` (`km.rs`, its
  layout asserts at `:247`). A host test (as `fwinit.rs:388-404`): knob off,
  the init bytes equal today's; knob on, the five words are set.
- `driver/rust/src/gpu/pvr/hwperf.rs` (new): `drain(k, out)` does (c) and
  returns compact records `{type, ordinal, cr_ts, ext_job_ref, dm, ufo_ev,
  ufo_addr, ufo_value}`; host tests on a buffer laid out by hand (a wrap, a
  bad signature, an ordinal gap). `extern "C" yd_pvr_hwperf_drain`.
- `driver/rust/src/gpu/pvr/render.rs`: write the kick's sequence number into
  `ui32ExtJobRef` of the geometry and fragment command headers (after the
  zeroing at `:1130`). Owned by #325 now: a two-line change to take after it
  releases the file, or skip it and match packets to kicks by order and UFO
  address.
- `driver/src/pvr_fw_boot.c`: `| YD_PVR_FWINIT_HWPERF` under the Kconfig in
  `pvr_fw_build_structs` (`:412-425`); `driver/src/pvr_render.c`:
  `pvr_render_hwperf_drain()` and a `pvr_render_hwperf_clock()` that reads
  `cntvct` and `CR_TIMER` back to back (as `pvr_render_perf_read`,
  `:334-343`).
- `driver/tools/pvrlab/gl_test.c` case 216 (`:20345-20525`): under
  `GL_TEST_PVR` and the knob, take the clock pair before and after the
  frames, drain after, and print per kick, joined on `ext_job_ref` = seq with
  the kick stamps (`kick_times.rs`, needs `YAKOGL_DRV_GPU_PVR_KICK_TIMES`;
  its ring holds 8 kicks, `kick_times.rs:30-31`, so the post times of the
  last 8 kicks of a frame, or a larger ring under the knob):

```
gl_test: case 216 hwperf: <n> packets, <d> dropped, cr0 <x> cntvct0 <y>, slope <ns/tick>
gl_test: case 216 hwperf kick <seq>: post->TA <us>, TA <us>, TA->3D <us>, 3D <us>, 3D->fence <us> (bg <us> after post, ufo fail <k>, csw <k>, pwr <k>)
gl_test: case 216 hwperf: median post->TA <us>, TA <us>, TA->3D <us>, 3D <us>, 3D->fence <us> over <n> kicks
```

where post = `t_posted`; TA = `TAKICK`..`TAFINISHED`; 3D =
`3DKICK`..`3DFINISHED`; 3D->fence = `3DFINISHED` to the UFO `UPDATE` of the
fragment fence (and to the host's first read of it, `t_frag`); bg = the
first `BGSTART` after the post.

## (e) What each reading of the 0.9 ms predicts

The 0.9 ms is between post and the geometry fence (`am62p-slow-terrain.md`,
the kick table and the paragraph after it). Each reading predicts a
different place for it in the per-kick line:

| hypothesis | post->TA | TA | other | decided by |
|---|---|---|---|---|
| H1: firmware take-up of a KCCB command (background task scheduled late, the cost candidate 4 of `am62p-slow-terrain.md` would amortise) | ~0.9 ms | small (~0.2 ms) | `bg` ~0.9 ms after post, `BGSTART`->`TAKICK` small | median post->TA >= 0.7 ms with `bg` within 0.1 ms of it |
| H2: the kick waits for a fence (a geometry fence check on an earlier kick's fragment UFO, e.g. the frame-set or HWRT reuse) | ~0.9 ms | small | UFO `CHECK_FAIL` on the kick's geometry CCB, then `CHECK_SUCCESS` right after the previous kick's fragment `UPDATE` | ufo fail >= 1 a kick; `TAKICK(N)` within ~50 us of `3DFINISHED(N-1)` |
| H3: the firmware serialises TA of kick N behind 3D of kick N-1 (no overlap, same render context) | ~ previous kick's 3D left | small | no UFO fail; `TAKICK(N)` after `3DFINISHED(N-1)` | TA->3D gaps small and `TAKICK(N) - 3DFINISHED(N-1)` < 50 us on every kick |
| H4: the TA itself is long (region headers or PM setup of a level-1-sized target, `am62p-slow-terrain.md` first row) | small | ~0.9 ms | perf counters' TA busy should then be high too | median TA >= 0.7 ms |
| H5: context store (CSW) around kicks, the medium profile (`fwinit.rs:180`) | grows by the CSW time | - | `CSW_START`/`FINISHED` per kick | csw >= 1 a kick with a duration near 0.9 ms |
| H6: a power transition between kicks (APM latency 5 ms, `driver/zephyr/Kconfig:26-35`) | power-up time | - | `PWR_CHG`/`GPU_STATE_CHG` between kicks | pwr >= 1 a kick; unlikely with kicks 1.1 ms apart and a 5 ms latency |
| H7: the fence write is late (3D done, the UFO written late or seen late by the host) | small | small | 3D->fence ~0.9 ms | median 3D->fence >= 0.7 ms (this would sit after, not before, the geometry fence: a check of the attribution itself) |

The board record values that decide: the median line's five numbers, the
counts in parentheses, and `dropped` 0 (else the medians are of a sample).
The sum post->TA + TA + TA->3D + 3D + 3D->fence over a frame should come
back to case 216's own `ms a frame` (`gl_test.c:20493-20501`) less the
host's share, within the 0.32 us tick: if it does not, the join of packets to
kicks is wrong.

## (f) Risks

- **Firmware support.** The comment at `fwinit.rs:246-248` shows the
  firmware reads the filter and emits a boot packet, so the producer is
  built in. Whether it honours a filter set before boot (route 1) or only
  `HWPERF_UPDATE_CONFIG` (route 2) is **[knowledge]**-level: the DDK sets
  both at init in its server (`RGXSetupFwOsInit`, not vendored). First
  board step: boot with the knob, look for the boot packet (a `FWACT`
  `FEATURES` event, `include/rgx_hwperf_common.h:1101`, is likely) at
  `WIdx` > 0 with no gl_test work.
- **Firmware fault.** A filter with no buffer faults at boot
  (`fwinit.rs:246-248`); the init must write the buffer address and size
  before the filter, and the knob off must leave all three 0.
- **Layout.** The struct offsets are pinned by the gate once in `fwif.rs`
  (a); the packet structs are only read by the host, from the DDK 25.2
  headers that match our firmware (`UPSTREAM`).
- **Buffer size.** 2 MB for one case 216 run (a); a full glmark2 run needs
  `RIdx` handling (c). A full buffer drops packets (`DropCount`, the ordinal
  gaps), it does not stop the GPU.
- **Cost of HWPerf itself.** Each packet is a firmware write of about 90
  bytes to uncached memory plus its bookkeeping; with about 10 packets a
  kick, a few microseconds a kick are plausible (**[knowledge]**, no figure
  in the tree). Measured, not assumed: case 216's `ms a frame` with the
  knob on against off, same image; more than ~2% (about 0.9 ms a frame of
  46) means the probe moves what it measures, and route 2 with a narrower
  filter (HW kick/finish and UFO only) is the fallback.
- **The clock.** Raw `CR_TIMER` against the firmware converting, and
  `CR_TIMER` against `SOC_TIMER` (b): the first-run check decides it before
  any number is read.
- **Our cache and the shared line.** (c): no `RIdx` write during a capture.

## Not done here

No code, no board run (contract of item pvr-hwperf-study, plan #329).
