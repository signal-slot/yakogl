# PowerVR firmware trace: enable it and decode it

Target: BXS-4-64 with the vendor firmware (DDK 25.2) booted by our driver,
bare-metal Zephyr or the pvrlab Linux module. The firmware writes a trace of its
own activity (kicks, HWR, power, MMU faults) into a ring in shared memory; this
note says how our driver enables it and how to read it, so that a kick that
hangs can be located without the Linux `pvrsrvkm` debugfs node.

The layouts are the vendored DDK's, in `third_party/ti-img-rogue-ddk-25.2`
(`include/rogue/rgx_fwif_shared.h` for the trace structs and log groups,
`include/rgx_fwif_sf.h` for the string-format table). The earlier version of
this note described the open-source `drm/imagination` structs
(`rogue_fwif_tracebuf`); that layout is not the one the firmware we boot uses.

## 1. The structures (`RGXFWIF_TRACEBUF`)

```
RGXFWIF_TRACEBUF
    ui32LogType                     log groups plus the TRACE bit
    sTraceBuf[RGXFW_THREAD_NUM]     one RGXFWIF_TRACEBUF_SPACE per FW thread
    ui32TraceBufSizeInDWords        ring length, RGXFW_TRACE_BUF_DEFAULT_SIZE_IN_DWORDS = 12000
    ui32TracebufFlags               0

RGXFWIF_TRACEBUF_SPACE
    ui32TracePointer                FW write index in dwords (also the oldest entry once wrapped)
    ui32WrapCount                   how many times the ring has wrapped
    pui32RGXFWIfTraceBuffer         FW address of the thread's ring
    sAssertBuf                      RGXFWIF_FILE_INFO_BUF: szPath[200], szInfo[200], ui32LineNum
```

`RGXFW_THREAD_NUM` is 1 unless `RGXFW_META_SUPPORT_2ND_THREAD` is defined, which
it is not for this RISC-V firmware, so there is one ring. There is no CPU-side
pointer in this layout: the host finds the ring by the address it allocated.

The offsets of the fields our code uses are pinned in
`driver/tools/offsetof/ddk_abi_pins.h`.

## 2. Enabling the trace (what `pvr_fw_boot.c` does)

All of this is done before the firmware core is released from reset, because the
firmware reads the control object during its own init and starts writing at once.

1. Allocate the control object (`RGXFWIF_TRACEBUF`) and the ring
   (`RGXFW_TRACE_BUF_DEFAULT_SIZE_IN_DWORDS * 4` bytes) from the firmware heap.
   Zero the ring.
2. Fill the control object: `sTraceBuf[0].pui32RGXFWIfTraceBuffer` with the
   ring's firmware address, `ui32TracePointer = 0`, `ui32WrapCount = 0`,
   `ui32TraceBufSizeInDWords = 12000`, `ui32TracebufFlags = 0`, and
   `ui32LogType`.
3. Point `RGXFWIF_SYSINIT.sTraceBufCtl` at the control object.
4. Flush the control object and ring from the CPU cache (the carveout is cached).

The log groups live only in `ui32LogType`. The bit values are
`RGXFWIF_LOG_TYPE_GROUP_*` in `rgx_fwif_shared.h`: `TRACE` is bit 0 and enables
the ring, `MAIN` 0x2, `MTS` 0x4, `CLEANUP` 0x8, `CSW` 0x10, `BIF` 0x20,
`PM` 0x40, `RTD` 0x80, `SPM` 0x100, `POW` 0x200, `HWR` 0x400, and so on up to
`MASK`. These bit positions are a separate namespace from the group IDs inside a
trace entry (section 3), and the open-source firmware numbers them differently;
using the wrong header was the mistake that once made this a no-op.

Our default is every group except `MTS`, `POW` and `DEBUG`
(`RGXFWIF_LOG_TYPE_MASK & ~DEBUG & ~(MTS | POW)`). MTS and POW log a
kick/complete pair and a power check on every idle poll, several entries per
millisecond with nothing happening. The ring wraps, so at a stall it held a
millisecond of idle chatter instead of the kicks and faults that led up to it.
Add `RGXFWIF_LOG_TYPE_GROUP_POW` back only when debugging power handling.

Optional: `RGXFWIF_INICFG_ASSERT_ON_HWR_TRIGGER` (bit 20 of `RGXFWIF_SYSDATA`
`.ui32ConfigFlags`) makes the firmware write `RGXFW_SF_MAIN_ASSERT_FAILED`
into the trace, with `sAssertBuf` filled, the moment a hardware recovery
triggers, which marks the lockup exactly. `RGXFWIF_INICFG_ASSERT_ON_OUTOFMEMORY`
(bit 13) does the same for PM/freelist exhaustion.

After boot the mask can be changed only by writing `ui32LogType`, flushing it,
and posting a log-type update command on the KCCB; with no working KCCB, set the
final mask before boot.

## 3. Entry layout

The ring is circular, in dwords. `ui32TracePointer` is the write index and
therefore the oldest entry once the ring has wrapped, so reading forward from it
gives chronological order. An entry is:

```
[0]       SFID           RGXFW_LOG_VALIDID(id): (id & 0xFFF00000) == 0x70000000
[1]       timestamp, bits 63..32
[2]       timestamp, bits 31..0
[3..3+N)  N parameters, N = RGXFW_SF_PARAMNUM(id)
```

The stride is `3 + N`. The timestamp keeps its low 48 bits
(`RGXFWT_TIMESTAMP_TIME_CLRMSK` clears the top 16, which are debug info). The
SFID encodes, in `rgx_fwif_sf.h`: the id number in the low bits, the group id
(`RGXFW_SF_GID`, bits 12-15 plus bit 7 as bit 4 of the group), the parameter
count (`RGXFW_SF_PARAMNUM`, bits 16-19) and the marker `0x70000000`. The format
string for an id is found by looking it up in `RGXFW_LOG_SFIDLIST`;
`RGXFW_SF_MAIN_ASSERT_FAILED` ends the trace and the reader prints
`sAssertBuf` instead.

## 4. Decoding on the target: `pvr_fw_trace_decode`

`driver/src/pvr_fw_trace_decode.[ch]` decodes the ring in the driver. It
includes the vendored `rgx_fwif_sf.h` and builds its format table and group
names from the DDK's own X-macros (`RGXFW_LOG_SFIDLIST`, `RGXFW_LOG_SFGROUPLIST`),
so it cannot drift from the firmware's table.

```c
void pvr_fw_trace_dump(const RGXFWIF_TRACEBUF *tb, uint32_t *const *ring_cpu,
                       uint32_t num_threads, pvr_fwt_emit_fn emit, void *ctx);
uint32_t pvr_fw_trace_dump_one(const RGXFWIF_TRACEBUF_SPACE *space,
                               const uint32_t *ring_cpu, uint32_t ring_dwords,
                               uint32_t thread, pvr_fwt_emit_fn emit, void *ctx);
```

`ring_cpu[t]` is the CPU pointer to thread t's ring (the firmware only knows its
firmware address). Each line is `T<t>-<GROUP>> [<timestamp>] <message>`; an id
that is not in the table is printed raw as `SFID=0x... (N args): ...`, and an
assert prints `ASSERTION <info> failed at <path>:<line>`. `emit == NULL` uses
`printf`. The caller must CPU-invalidate the control object and the ring first
when they are in a cached mapping, since the firmware wrote them.

The self-test rig calls it from `pvr_fw_boot.c` when built with
`-DPVR_EARLY_TRACE`: it waits about 40 ms after the TA/3D kick (the geometry has
run and stalled, but the roughly 125 ms watchdog HWR has not yet spammed the
ring over the early lines) and dumps through `LOG_INF("FWT| ...")`. The Linux
pvrlab module builds the decoder (`tools/pvrlab/Kbuild`).

## 5. Decoding offline: `tools/sfdecode`

The render and boot paths also log the raw ring tail as lines of six hex dwords
(`pvr_render.c` after a render, and `pvr_fw_boot.c` on a boot timeout and early
after a kick). `driver/tools/sfdecode/sfdecode.py` decodes such a log without
rebuilding: it reads the SFID list from the vendored `rgx_fwif_sf.h`
(or the header named on the command line, or `SFID_HEADER`), rebuilds each id
from the list's group, number and parameter count, scans the dwords for a valid
marker, and prints `[<dword index>] <NAME>: <formatted message>`.

```sh
grep 'pvrgpu:  t\[' ours_dmesg.txt | driver/tools/sfdecode/sfdecode.py
```

It decodes from the start of the dump, not from the trace pointer, so a ring
that has wrapped reads in wrapped order; use the in-driver decoder for a
chronological view.

## 6. What to look for after a kick times out

- `Kick TA` with no `TA finished` (MAIN group): the geometry/tiling phase never
  completed, which is the hang. Likewise `Kick 3D` and `3D finished`.
- `ASSERTION ... failed at <path>:<line>`: the firmware hit an assert; with
  `RGXFWIF_INICFG_ASSERT_ON_HWR_TRIGGER` it fires exactly at the lockup.
- BIF lines, or a timestamp with the MMU-fault debug bit set: a bad GPU VA in
  the TA or PM data (freelist, RTA, VDM state), the most common cause of a
  first-bringup TA stall.
- PM and SPM lines such as the out-of-memory message: freelist,
  partial-render or tiler-heap exhaustion.
- HWR group lines: the firmware's own lockup detection kicked in; what is
  immediately above them is the guilty data master's last progress.
- A failing `UFO Check`: a fence the kick waits on was never signalled, so the
  data master is blocked on a dependency rather than running.

The entries just before the HWR or assert lines are the firmware's last forward
progress; that is where the hang is.
