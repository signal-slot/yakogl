<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): what panthor logs on reset, fault and timeout

Date 2026-11-07. Plan #327 item `panthor-kmsg-note`, for the NanoPC-T6 dEQP-GLES2
runner and its tally (issue #20, `driver/docs/nanopc-t6-mali.md`). It is a
reading of the Linux kernel source of the `panthor` driver; host research
only, no file outside this note changed, no board run, `gh` was not run.

## Source read

Linux **v6.18.55** (stable tree, tag `v6.18.55`, commit
`725bd2f3c81d54edccb66fead01d4c0c222e2231`), `drivers/gpu/drm/panthor/`,
`include/uapi/drm/panthor_drm.h`, `include/drm/drm_print.h` and
`drivers/gpu/drm/drm_print.c`. It is the version of the NanoPC-T6's Armbian
kernel (`6.18.55-current-rockchip64`). Armbian's `current-rockchip64` is the
stable tree plus its own patch set; this note did not read that set, so a
patch to panthor in it would not show here. **Pin on the board:**
`modinfo panthor` and `uname -r` per session, and a `grep` of the strings
below in `/lib/modules/$(uname -r)/kernel/drivers/gpu/drm/panthor/panthor.ko*`
(`strings` on the decompressed module) if a message is missing.

Line numbers are of that tag. The tree is in
`$CONNECTED_DEPS_DIR/linux-v6.18.55` (a sparse checkout of `drivers/gpu/drm/panthor`
and `include/uapi/drm`).

## What the runner can rely on

1. **A successful reset logs nothing.** `panthor_device_reset_work`
   (`panthor_device.c:127`) has no message on the success path; the only line
   it can print is the failure at `:154`. A reset is announced only by the
   *cause* line that scheduled it (section 2), and that line is printed
   *after* `panthor_device_schedule_reset()` already queued the work, so a
   cause line is evidence that a reset ran or was queued, not a count of
   resets. Several causes inside one reset window give several lines and one
   reset: `reset.pending` is a 0 to 1 flag (`panthor_device.h:267`), cleared at
   the end of the work.
2. **There is no reset counter anywhere** in 6.18.55 panthor: no debugfs file,
   no sysfs attribute, no fdinfo key, no tracepoint, no devcoredump (`grep -rn
   'devcoredump\|trace' drivers/gpu/drm/panthor/` finds none). The runner's
   "panthor reset count" is therefore the **count of cause lines** of class
   `reset` in section 2, per chunk; define it so and say so in the report.
3. **Every reset in `reset_work` is a slow reset.** The work calls
   `panthor_fw_pre_reset(ptdev, true)` (`:140`, `on_hang` true), which sets
   `reset.fast = false`; `panthor_fw_post_reset` then reloads all firmware
   sections from the cached image (no new `request_firmware`, so no firmware
   file messages). "Fast" resets exist only on the suspend/resume path.
4. **A job timeout does not reset the GPU.** `queue_timedout_job` marks the
   group timed out and has the scheduler terminate it (section 3). A hung
   dEQP case costs its own group, the GPU keeps running unless a firmware
   fault follows.
5. **A failed reset makes the render node unusable until reboot** (`:154`
   `panthor_device_unplug`): opens and ioctls then fail with `ENODEV`, the
   board and ssh stay up. A chunk after that cannot draw; the runner must
   stop, not retry (`rmmod`/`modprobe` of panthor is the board's recovery,
   which the runner must not do by itself).
6. Messages are printed with `drm_*`, so every line in dmesg looks like

   ```
   [  123.456789] panthor fb000000.gpu: [drm] *ERROR* Soft reset timeout
   [  123.456789] panthor fb000000.gpu: [drm] GPU Fault 0x00000088 (...) at 0x...
   ```

   `*ERROR* ` is added by `drm_err` only (`drm_print.h:627`); `drm_warn` and
   `drm_info` have `[drm] ` and no level word. `fb000000.gpu` is the RK3588
   device name (read it from `/sys/class/drm/card*/device` on the board; do
   not hard-code it). `drm_WARN_ON(x)` prints `panthor <dev>: [drm]
   drm_WARN_ON(x)` inside a `------------[ cut here ]------------` block with
   `WARNING: ... at drivers/gpu/drm/panthor/<file>.c:<line> <function>+0x...
   [panthor]` and a call trace; the level of `dev_warn`/`drm_warn` lines is
   KERN_WARNING (4), of `drm_err` KERN_ERR (3), of `drm_info` KERN_INFO (6).
   Several lines are one `drm_warn` with embedded newlines
   (`CS_FATAL`, `CS_FAULT`, the MMU fault): in `dmesg` they print as one
   record, in `journalctl -k` as several, in `dmesg --raw`/`/dev/kmsg` as one
   record with `\x0a`. Match each line of them separately.

Regex convention below: `PFX` is `panthor \S+: \[drm\] ` and `E` is
`\*ERROR\* `, so `PFX E Soft reset timeout` means
`panthor \S+: \[drm\] \*ERROR\* Soft reset timeout`. Regexes are ERE and drop
the timestamp.

## 1. Quick classification

| class | meaning | reset | lines |
| --- | --- | --- | --- |
| `reset-cause` | scheduled a reset (`panthor_device_schedule_reset`) | yes | 2.1 |
| `reset-fail` | the reset work failed: device unplugged | yes, fatal | 2.2 |
| `hw-timeout` | power, cache or MCU timeout, no reset by itself | no | 2.3 |
| `gpu-fault` | GPU fault IRQ | no | 5 |
| `mmu-fault` | AS page fault on a VM | no (group dies) | 4 |
| `cs-fault` | CSF stream fault or fatal | `CS_UNRECOVERABLE` only | 3 |
| `job-timeout` | job or progress timeout | no (group dies) | 3 |
| `fw-load` | firmware image problems | only at probe | 6 |
| `info` | probe-time facts | no | 7 |

## 2. Reset: `panthor_device_schedule_reset` and `panthor_device_reset_work`

### 2.1 What schedules a reset (the cause lines)

`panthor_device_schedule_reset` (`panthor_device.h:267`) queues the work only if
none is pending and the device is `ACTIVE`; during suspend a pending flag
forces a slow reset at the next resume (`panthor_device.c:495`). It prints
nothing itself. Its callers, all in 6.18.55:

| format string | file:function (line) | trigger | reset | regex |
| --- | --- | --- | --- | --- |
| `Flush caches timeout` | `panthor_gpu.c:panthor_gpu_flush_caches` (294) | `GPU_CMD` flush (L2/LSC/other) not acked in 100 ms | yes, scheduled | `PFX E Flush caches timeout` |
| `AS_ACTIVE bit stuck\n` | `panthor_mmu.c:wait_ready` (520) | AS command pending for 100 ms (MMU stuck) | yes, scheduled | `PFX E AS_ACTIVE bit stuck` |
| `FW ping timeout, scheduling a reset` | `panthor_fw.c:panthor_fw_ping_work` (1361) | the MCU did not ack `GLB_PING` in 100 ms; the ping runs every 12 s (`PING_INTERVAL_MS`) | yes, scheduled | `PFX E FW ping timeout, scheduling a reset` |
| `Invalid state on CSG %d (state=%d)` | `panthor_sched.c:csg_slot_sync_state_locked` (1269) | the firmware reports a CSG state the driver does not know (firmware state corruption) | yes (`:1282`) | `PFX E Invalid state on CSG [0-9]+ \(state=-?[0-9]+\)` |
| `CS_FATAL: pid=%d, comm=%s\n` then `CSG slot %d CS slot: %d\nCS_FATAL.EXCEPTION_TYPE: 0x%x (%s)\nCS_FATAL.EXCEPTION_DATA: 0x%x\nCS_FATAL_INFO.EXCEPTION_DATA: 0x%llx\n` | `panthor_sched.c:cs_slot_process_fatal_event_locked` (1379, 1395) | a CS fatal event; **a reset only when the exception type is `CS_UNRECOVERABLE`** (`:1386`), otherwise only the group dies | only for `CS_UNRECOVERABLE` | see 3 |
| `CSG %d update request timedout` | `panthor_sched.c:csgs_upd_ctx_apply_locked` (1897) | a CSG request (status update, priority, start, resume, suspend, terminate) not acked in time | yes in the four callers that check the result: `tick_ctx_init` (2075), `tick_ctx_apply` (2248, 2313; 2294 is a failed `group_bind_locked`); the suspend path (`panthor_sched_suspend`) escalates instead (2780) | `PFX E CSG [0-9]+ update request timedout` |

The first three rows are where the cause line and the `schedule_reset` call
are in the same function; the last three are the scheduler's. A tick that fails in `tick_ctx_apply` at `:2294` (bind
failure) prints no message of its own; it is visible only as the reset's
effects.

### 2.2 `reset_work` and what it prints

`panthor_device_reset_work` (`panthor_device.c:127`) runs `panthor_sched_pre_reset`,
`panthor_fw_pre_reset(true)`, `panthor_mmu_pre_reset`, `panthor_gpu_soft_reset`,
`panthor_gpu_l2_power_on`, `panthor_mmu_post_reset`, `panthor_fw_post_reset`,
`panthor_sched_post_reset`. Messages it can reach:

| format string | file:function (line) | trigger | reset | regex |
| --- | --- | --- | --- | --- |
| `Soft reset timeout` | `panthor_gpu.c:panthor_gpu_soft_reset` (333) | no `RESET_COMPLETED` IRQ in 100 ms | inside the reset: the reset goes on, the MCU boot then probably fails | `PFX E Soft reset timeout` |
| `timeout waiting on %s:%llx power transition` | `panthor_gpu.c:panthor_gpu_block_power_off` (158, 169), `panthor_gpu_block_power_on` (201) | `PWRTRANS` bits stay set; `%s` is `L2` (and `tiler`/`shader` on the suspend path); mask in hex | inside a reset or suspend; no new reset | `PFX E timeout waiting on [A-Za-z0-9_]+:[0-9a-f]+ power transition` |
| `timeout waiting on %s:%llx readiness` | `panthor_gpu.c:panthor_gpu_block_power_on` (213) | `READY` bits do not come up | as above | `PFX E timeout waiting on [A-Za-z0-9_]+:[0-9a-f]+ readiness` |
| `Failed to boot MCU (status=%s)` | `panthor_fw.c:panthor_fw_start` (1084) | MCU did not raise `GLOBAL_IF` in 1 s; `%s` is `disabled`, `enabled`, `halt`, `fatal` or `unknown` | the reset failed | `PFX E Failed to boot MCU \(status=[a-z]+\)` |
| `FW %s reset failed` | `panthor_fw.c:panthor_fw_post_reset` (1171) | `panthor_fw_start` failed; `%s` is `fast` or `slow` (always `slow` from `reset_work`) | the reset failed | `PFX E FW (fast\|slow) reset failed` |
| `Failed to stop MCU` | `panthor_fw.c:panthor_fw_stop` (1099) | `MCU_STATUS` is not `DISABLED` after 100 ms | inside the reset | `PFX E Failed to stop MCU` |
| `Failed to boot MCU after reset, making device unusable.` | `panthor_device.c:panthor_device_reset_work` (154) | `panthor_fw_post_reset` returned an error | **reset failed; `panthor_device_unplug`: the render node is gone until reboot or module reload** | `PFX E Failed to boot MCU after reset, making device unusable\.` |

`Failed to cleanly suspend MCU` (`panthor_fw.c:panthor_fw_pre_reset`, 1128, a
`drm_warn`) is **not** reached from `reset_work` (`on_hang` is true there); it
is the suspend path's: regex `PFX Failed to cleanly suspend MCU`. A
`Fast reset failed, trying a slow reset` (`panthor_device.c:panthor_device_resume`,
500, `drm_err`; regex `PFX E Fast reset failed, trying a slow reset`) is the
resume path after a graceful halt failed to restart; it is not a GPU hang.

Runtime suspend and resume (autosuspend delay 50 ms,
`panthor_device.c:276`) power the GPU down and up with **no message** (a
soft reset in `panthor_gpu_suspend` included). A silent reset can happen
between two cases of an idle runner; do not read "no line" as "no reset" for
the idle gaps, only for the work between submits.

## 3. Job and group timeout, CS faults: `panthor_sched.c`

Timeouts: the DRM scheduler timeout of a queue is 5 s (`JOB_TIMEOUT_MS`,
`:91`); the firmware's progress timer is `PROGRESS_TIMEOUT_CYCLES` = 5 x 500 x
1024 x 1024 cycles shifted right by 10 (`panthor_fw.c:31`), meant as about
5 s (the counter's rate was not checked). Neither resets the GPU.

| format string | file:function (line) | trigger | reset | regex |
| --- | --- | --- | --- | --- |
| `job timeout: pid=%d, comm=%s, seqno=%llu\n` | `queue_timedout_job` (3317) | DRM scheduler: a job of a queue did not signal in 5 s; the group is flagged `timedout`, the tick (or `group_queue_work(term)`) kills it | **no**: the function returns `DRM_GPU_SCHED_STAT_RESET` and calls no panthor reset function | `PFX job timeout: pid=[0-9]+, comm=.*, seqno=[0-9]+` |
| `CSG_PROGRESS_TIMER_EVENT: pid=%d, comm=%s\n` | `csg_slot_process_progress_timer_event_locked` (1669) | the firmware's progress timer fired on the CSG; group flagged `timedout` | no | `PFX CSG_PROGRESS_TIMER_EVENT: pid=[0-9]+, comm=.*` |
| `CSG slot %d progress timeout\n` | same (1675) | same event, second line (printed even when the group is missing) | no | `PFX CSG slot [0-9]+ progress timeout` |
| `CS_FATAL: pid=%d, comm=%s\n` | `cs_slot_process_fatal_event_locked` (1379) | a command stream hit a fatal exception; the queue is added to the group's `fatal_queues` | see 2.1 | `PFX CS_FATAL: pid=[0-9]+, comm=.*` |
| `CSG slot %d CS slot: %d` / `CS_FATAL.EXCEPTION_TYPE: 0x%x (%s)` / `CS_FATAL.EXCEPTION_DATA: 0x%x` / `CS_FATAL_INFO.EXCEPTION_DATA: 0x%llx` | same (1395) | the details of the event, one `drm_warn` of four lines | `CS_UNRECOVERABLE` only | `CS_FATAL\.EXCEPTION_TYPE: 0x[0-9a-f]+ \(([A-Z_0-9]+\|Unknown exception type)\)` |
| `CS_FAULT: pid=%d, comm=%s\n` | `cs_slot_process_fault_event_locked` (1444) | a command stream fault (not fatal: `CS_INHERIT_FAULT`, `CS_BUS_FAULT`, ...) | no | `PFX CS_FAULT: pid=[0-9]+, comm=.*` |
| `CSG slot %d CS slot: %d` / `CS_FAULT.EXCEPTION_TYPE: 0x%x (%s)` / `CS_FAULT.EXCEPTION_DATA: 0x%x` / `CS_FAULT_INFO.EXCEPTION_DATA: 0x%llx` | same (1448) | details | no | `CS_FAULT\.EXCEPTION_TYPE: 0x[0-9a-f]+ \(([A-Z_0-9]+\|Unknown exception type)\)` |
| `Failed to extend the tiler heap\n` | `group_process_tiler_oom` (1512) | the heap could not grow for a reason other than `-ENOMEM` (invalid chunk arithmetic from the firmware): the queue is fatal | no | `PFX Failed to extend the tiler heap` |
| `CSG suspend failed, escalating to termination` | `panthor_sched_suspend` (2780) | a CSG suspend request timed out (during a reset's `panthor_sched_pre_reset`, or a suspend); the group becomes `timedout`, and `innocent` if it could still run | part of a reset or suspend | `PFX E CSG suspend failed, escalating to termination` |
| `Not enough AS (%d, expected at least 2)` | `panthor_sched_init` (4008) | probe only | no | `PFX E Not enough AS` |

`pid=` and `comm=` are the task that created the group (`group->task_info`),
so a dEQP run's lines carry `comm=deqp-gles2` (the binary's name, at most 15
characters); lines of another process (the compositor) have its comm. The
tally should count a line for the chunk only when `comm` is the runner's
binary, and report others separately.

The exception name `%s` is `panthor_exception_name` (`panthor_device.c:364`),
the table `panthor_exception_infos` with `PANTHOR_EXCEPTION(x)` names such as
`CS_UNRECOVERABLE`, `CS_BUS_FAULT`, `CS_INHERIT_FAULT`, `INSTR_INVALID_PC`,
`DATA_INVALID_FAULT`, `TRANSLATION_FAULT_0..4`, `PERM_FAULT_0..3`,
`ACCESS_FLAG_1..3`, `ADDR_SIZE_FAULT_*`, `MEM_ATTR_FAULT_*`; a code not in
the table prints `Unknown exception type`. Use `[A-Z_0-9]+` and keep the name
as the cause key.

## 4. MMU and address-space fault: `panthor_mmu.c`

| format string | file:function (line) | trigger | reset | regex |
| --- | --- | --- | --- | --- |
| `Unhandled Page fault in AS%d at VA 0x%016llX\n` `raw fault status: 0x%X\n` `decoded fault status: %s\n` `exception type 0x%X: %s\n` `access type 0x%X: %s\n` `source id 0x%X\n` | `panthor_mmu_irq_handler` (1723) | a GPU access to an unmapped VA or with the wrong permissions (the only MMU fault message; driver "does not handle VM faults"); `decoded` is `DECODER FAULT` or `SLAVE FAULT`; access `ATOMIC`, `READ`, `WRITE`, `EXECUTE` | no: the AS is disabled to kill the jobs, `vm->unhandled_fault` is set, the scheduler is ticked and the group's queues become `fatal` | `PFX E Unhandled Page fault in AS[0-9]+ at VA 0x[0-9A-F]{16}` and, for the cause, `exception type 0x[0-9A-F]+: ([A-Z_0-9]+\|Unknown exception type)` |
| `failed to unmap range %llx-%llx (requested range %llx-%llx)\n` | `panthor_vm_unmap_pages` (928) | a page-table unmap of a range returned less than asked (with a `drm_WARN_ON` trace before it); driver bug or corrupted table | no | `PFX E failed to unmap range [0-9a-f]+-[0-9a-f]+ \(requested range [0-9a-f]+-[0-9a-f]+\)` |
| `AS_ACTIVE bit stuck` | see 2.1 | | yes | |

`drm_dbg` lines `map: as=...`, `unmap: as=...` (`:919`, `:969`) print only with
`drm.debug` set; the runner must not set it.

The only address-space error that makes the GPU reset is `AS_ACTIVE bit stuck`.
A page fault shows as the six-line block, followed by a `CS_FATAL`/`CS_FAULT`
block of the same group when the firmware reports it, and by the dEQP case
failing with a lost context or an `EGL_CONTEXT_LOST`/`GL_OUT_OF_MEMORY` style
result in userspace (not kernel).

## 5. GPU fault IRQ: `panthor_gpu.c`

| format string | file:function (line) | trigger | reset | regex |
| --- | --- | --- | --- | --- |
| `GPU Fault 0x%08x (%s) at 0x%016llx\n` | `panthor_gpu_irq_handler` (63) | `GPU_IRQ_FAULT`; `fault_status & 0xFF` is the exception, `%s` its name, then the address register | no (a following timeout may reset) | `PFX GPU Fault 0x[0-9a-f]{8} \(([A-Z_0-9]+\|Unknown exception type)\) at 0x[0-9a-f]{16}` |
| `GPU Fault in protected mode\n` | same (68) | `GPU_IRQ_PROTM_FAULT` (protected mode is off on this board) | no | `PFX GPU Fault in protected mode` |
| `Coherency not supported by the device` | `panthor_gpu_coherency_init` (`panthor_device.c:40`) | probe only | no | `PFX E Coherency not supported by the device` |

Also in this file: `Flush caches timeout` (2.1), `Soft reset timeout` and the
`timeout waiting on ...` lines (2.2).

## 6. CSF firmware load and reload: `panthor_fw.c`

The firmware file is loaded once at probe (`panthor_fw_load`, `request_firmware`
of `mali_csffw.bin`, `CSF_FW_NAME`, under `arm/mali/arch10.8/` for the G610).
A reset does **not** read the file again; it copies the cached sections
(`panthor_reload_fw_sections`, no message). The following can therefore only
appear at module load:

| format string | file:function (line) | trigger | regex |
| --- | --- | --- | --- |
| `Failed to load firmware image '%s'\n` | `panthor_fw_load` (766) | file missing in `/lib/firmware/arm/mali/arch10.8/` | `PFX E Failed to load firmware image` |
| `Invalid firmware magic\n`, `Unsupported firmware binary header version %d.%d (expected %d.x)\n`, `Firmware image is truncated\n`, `Firmware too small\n`, `Firmware entry too long\n`, `Firmware corrupted, ...`, `Firmware contains interface with unsupported flags (0x%x)\n`, `Interface at 0x%llx must be shared`, `Firmware entry isn't 32 bit aligned, ...`, `Unsupported non-optional entry type %u in firmware\n`, `Shared interface region not found\n` | `panthor_fw_load`, `panthor_fw_load_section_entry`, `panthor_fw_load_entry`, `panthor_fw_binary_iter_read`, `panthor_fw_binary_sub_iter_init` (370-805) | broken or wrong-version image | `PFX E Firmware ` and the other prefixes of the list |
| `Firmware protected mode entry not be supported, ignoring` | `panthor_fw_load_section_entry` (537, `drm_warn`) | image has a protected-mode entry | `PFX Firmware protected mode entry not be supported` |
| `Firmware build info corrupt\n`, `Firmware's git sha is not NULL terminated\n`, `Firmware git sha: %s\n` | `panthor_fw_read_build_info` (662, 674, 679) | build-info entry | `PFX Firmware git sha: [0-9a-f]+` |
| `Invalid stream control interface input/output VA`, `Expecting identical CS slots`, `Invalid group control interface input/output VA`, `Expecting identical CSG slots`, `Firmware version is 0. Firmware may have failed to boot`, `Invalid global control interface input/output VA`, `Invalid number of control groups` | `panthor_init_cs_iface`, `panthor_init_csg_iface`, `panthor_fw_init_ifaces` (865-989) | the shared interface read after the first boot is wrong | `PFX E (Invalid \|Expecting identical \|Firmware version is 0)` |
| `failed to request job irq` | `panthor_fw_init` (1394) | probe | `PFX E failed to request job irq` |

For the runner: the firmware-load line set is a **probe-time** signal. If any
of them appears in a chunk's dmesg slice, the module was reloaded (by hand, or
the board rebooted) and the chunk is invalid. `Firmware git sha: ...` appearing
in a chunk's slice is the same sign. The MCU-boot-after-reset failures are in
2.2. A `drm_WARN_ON` backtrace in `panthor_fw.c` (`:296`, `:313`, `:345`,
`:356`, `:409`, `:620`, `:1281`, `:1307`) is a driver-side invariant, not a
fault of the GPU.

## 7. Probe-time lines (not faults)

A chunk's slice should contain none of them; their presence means a module
reload or a reboot. All `drm_info`:

| format string | file:function | regex |
| --- | --- | --- |
| `clock rate = %lu\n` | `panthor_device.c:panthor_clk_init` (64) | `PFX clock rate = [0-9]+` |
| `%s id 0x%x major 0x%x minor 0x%x status 0x%x` | `panthor_hw.c:panthor_hw_info_init` (101) | `PFX .* id 0x[0-9a-f]+ major 0x[0-9a-f]+ minor 0x[0-9a-f]+ status 0x[0-9a-f]+` |
| `Features: L2:%#x Tiler:%#x Mem:%#x MMU:%#x AS:%#x` | same (106) | `PFX Features: L2:` |
| `shader_present=0x%0llx l2_present=0x%0llx tiler_present=0x%0llx` | same (114) | `PFX shader_present=0x` |
| `using only 1st core group (%lu cores from %lu)\n` | `panthor_gpu.c:panthor_gpu_l2_power_on` (239, `drm_info_once`) | `PFX using only 1st core group` |
| `CSF FW using interface v%d.%d.%d, Features %#x Instrumentation features %#x` | `panthor_fw.c:panthor_fw_init_ifaces` (1000) | `PFX CSF FW using interface v` |
| `Firmware git sha: %s\n` | see 6 | |

## 8. Where else a reset or fault can be read

Everything below is the kernel as read in v6.18.55; nothing was run on the
board.

| source | what it shows | root | notes |
| --- | --- | --- | --- |
| `dmesg`, `/dev/kmsg`, `journalctl -k` | all lines above | `kernel.dmesg_restrict=1` needs root (or `CAP_SYSLOG`); Armbian's value is **pin on the board** (`sysctl kernel.dmesg_restrict`) | take the position before a chunk (`dmesg --since`/last `/dev/kmsg` sequence number) and read from it after; the ring buffer can wrap in a long chunk: read `/dev/kmsg` continuously (a follower per chunk) rather than `dmesg` at the end |
| `/proc/interrupts` | counts of the `job`, `mmu`, `gpu` IRQs of the GPU (`panthor_request_*_irq`) | no | a reset leaves a visible step in the `job` count (the firmware boot raises `GLOBAL_IF`) and in `gpu` (`RESET_COMPLETED`); not a count of resets, a cross-check only |
| `/sys/class/drm/card*/device/power/runtime_status`, `runtime_suspended_time`, `runtime_active_time` | the GPU's power state; a long `suspended` after a hang means the GPU idled, not that it recovered | no (readable) | silent suspend/resume is not a reset (2.2) |
| `/sys/class/devfreq/*.gpu/` (`cur_freq`, `trans_stat`) | clock only; `panthor_devfreq` | `trans_stat` readable | nothing about resets |
| `/sys/devices/platform/*.gpu/profiling` | `DEVICE_ATTR_RW` `profiling` (`panthor_drv.c:1647`): the profile mask (`PANTHOR_DEVICE_PROFILING_*`); fdinfo gets cycles and time when set | **write needs root**, read does not | not a reset indicator; a profiled run changes the GPU's work, do not turn it on for a conformance run |
| `/sys/kernel/debug/dri/<n>/gems`, `.../gpuvas` | `gems` (`panthor_drv.c:1565`): all live BOs with creator process; `gpuvas` (`panthor_mmu.c:2860`): VA map of each VM | **root** and a mounted debugfs | nothing about faults or resets. There is **no debugfs file for resets, groups or faults** in 6.18.55; the file names are as the DRM core creates them, **pin on the board** with `ls /sys/kernel/debug/dri/*/` |
| `/proc/<pid>/fdinfo/<fd>` of the render node (`/dev/dri/renderD128`) | `drm-engine-panthor`, `drm-cycles-panthor` (only with `profiling` set), `drm-maxfreq-panthor`, `drm-curfreq-panthor`, `drm-resident-memory`, `drm-active-memory` (`panthor_drv.c:1504`) | no for the own process, root for another | no fault or reset key. `drm-curfreq` is a way to see that the GPU is awake |
| `/sys/class/devcoredump/` | **empty for panthor in 6.18.55**: the driver has no `dev_coredumpv`/`devcoredump` call (a grep of `drivers/gpu/drm/panthor/` is empty); the directory exists only when other drivers create dumps | root to read the data files | do not expect a dump from a panthor hang; the runner may still list the directory and record "none" |
| `DRM_IOCTL_PANTHOR_GROUP_GET_STATE` (`panthor_sched.c:panthor_group_get_state`, 3707) | per group of the calling file: `state` bits `TIMEDOUT` (1), `FATAL_FAULT` (2), `INNOCENT` (4) and `fatal_queues` mask | no: any process that can open the render node (`render` group) and owns the group | **names differ from "innocent/guilty/fatal"**: there is no `guilty` bit. The mapping: *guilty* = `TIMEDOUT` without `INNOCENT` (its job hung or its suspend failed alone), *fatal* = `FATAL_FAULT` (a CS fatal, an MMU fault, or a tiler heap failure on this group), *innocent* = `TIMEDOUT`+`INNOCENT` and not `FATAL_FAULT` (set only where a CSG suspend request failed on a group that could still run, `panthor_sched.c:2790`). A group with any flag accepts no more jobs. The runner is the userspace driver (the Mali back end of yakogl); `deqp-gles2` cannot be asked, so the back end must call the ioctl after a failed submit or wait and print the state, or the runner reads only dmesg |
| the ioctl result and `errno` of `GROUP_SUBMIT` and the fence wait | `ENODEV` after an unplug (2.2); a failed wait of a killed group | no | the user-visible half of a chunk crash |

**Which need root:** dmesg (if restricted), debugfs, devcoredump data, the
`profiling` write, and `fdinfo` of another user's process. **Which do not:**
`/proc/interrupts`, `runtime_status`, `fdinfo` of one's own fd, the
`GROUP_GET_STATE` ioctl, `/sys/devices/.../profiling` read.

## 9. Per-chunk record (what the runner and tally can rely on)

For one chunk, the record is, in this order of reliability:

1. **ssh and render node still alive afterwards**: `ssh` returns, and
   `test -c /dev/dri/renderD128` and a trivial open. Failure of the render
   node with ssh alive is the `Failed to boot MCU after reset` state (2.2).
2. **Cause lines by class** in the dmesg slice of the chunk (sections 2 to 5).
   The class names of section 1 are the stable keys; the counters are:
   `reset-cause` = number of lines of the rows of 2.1 (CS_FATAL counted
   only when its `EXCEPTION_TYPE` is `CS_UNRECOVERABLE`), `reset-fail` = lines of
   2.2 with the unplug text, `job-timeout`, `mmu-fault`, `gpu-fault`, `cs-fault`
   likewise.
3. **Probe lines** (6, 7) in the slice: the chunk is invalid (module
   reloaded or reboot); nothing else of it can be trusted.
4. **`WARNING:` blocks** naming `panthor`: a driver invariant; record the file
   and line, `WARNING: CPU: [0-9]+ PID: [0-9]+ at drivers/gpu/drm/panthor/[a-z_]+\.c:[0-9]+ `.
5. A **hung kernel thread** report from elsewhere (`task .* blocked for more
   than`, `rcu: INFO`) in the slice: the board is probably unusable over ssh
   soon; stop.
6. `/proc/interrupts` deltas of the `job`, `mmu`, `gpu` lines (cross-check).

A clean chunk has no line of classes 1 to 5 in the slice. A chunk with only
`job-timeout` and `cs-fault` lines is a case that killed its own group and did
not take the GPU with it: the following chunk can run; with a `reset-cause`
line the groups of every other client are stopped and restarted, or killed by a failed suspend (`innocent`), so a second
process on the GPU (the compositor) must not be running, or the tally should
name it.

## 10. Unread and to pin on the board

- Armbian's patches to panthor (section "Source read").
- The exact `dmesg` device name and the `dmesg_restrict` value.
- Whether `drm_info` lines are on at the log level the console shows (they are
  KERN_INFO; the kernel ring buffer keeps them regardless).
- The `CS_FATAL`/`CS_FAULT` exception names a G610 produces (the table is
  generic; the first board record names the real ones).
- Whether the board's `panthor.ko` carries the string `Failed to boot MCU after
  reset, making device unusable.` (a cheap check that section 2.2 matches).
