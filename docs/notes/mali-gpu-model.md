<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The Mali kernel side on a GPU other than the T6's G610: what it reads, the G310 mock

The Mali kernel side (`driver/rust/src/gpu/mali/`, the `board::nanopc_t6_*`
glue) was written for the Mali-G610 of the RK3588 as the NanoPC-T6 reads it
(`driver/docs/nanopc-t6-mali.md`, "The GPU, the SoC, the board"). For the
i.MX 95 port (Mali-G310) the values that differ between Valhall CSF GPUs are
read from the GPU at probe, as panthor reads them, and the host tests run the
GPU's sequences against a G310 mock beside the G610 one. This page says what
is read, what the G310 mock rests on, and what is still the G610's or the
RK3588's.

panthor is Linux v6.18's `drivers/gpu/drm/panthor/`, read for the hardware's
behaviour and cited by file and line at tag v6.18 (read on 2026-11-20 from
`raw.githubusercontent.com/torvalds/linux/v6.18/drivers/gpu/drm/panthor/`;
MIT option of its licence, the design page's decision 1). Nothing is copied.

## What is read from the GPU

| Value | Where it is used | Before | panthor |
|---|---|---|---|
| `GPU_ID` arch major, product major | `gpu::mali::identify` (`probe`, `board::nanopc_t6_gpu::gpu_up`): the model, or stop 6 `UnsupportedGpu` | `== 0xa8670005` | panthor_hw.c:11-59 `get_gpu_model_name` (`GPU_PROD_ID_MAKE(arch major, product major)`, the arch-10 cases at :21-28), read at :65 |
| `GPU_ID` arch major, minor | `gpu::mali::fw_arch`: the firmware's directory, `arm/mali/arch<maj>.<min>/` | `arch10.8` in comments | panthor_fw.c:745-748 `panthor_fw_load` |
| `MMU_FEATURES` VA bits 7:0, PA bits 15:8 | `pgtable::Shape`: levels, root size, VA and PA range checks of `Tables`; `mmu::transcfg`'s input size; `nanopc_t6_mmu::as0_up` (AS0), `nanopc_t6_submit` (AS0 reopened, AS1) | `VA_BITS` 48, `PA_BITS` 40, 4 levels; the check refused all but 48/≥40 | panthor_mmu.c:2288-2289 and :2345-2346 `panthor_vm_create` (`ias`/`oas` to io-pgtable), :704 and :761-765 `panthor_vm_active` (`AS_TRANSCFG_INA_BITS(55 - va_bits)`); panthor_gpu.c:120-121 (the DMA mask from the PA bits) |
| `L2_PRESENT` | `l2_power_on`: the mask powered (the first L2), `NoL2` when 0 | read already | panthor_gpu.c:227-248 `panthor_gpu_l2_power_on` (powers L2 mask 1, 20 ms, :247) |
| `SHADER_PRESENT` | `csf::global_init`'s `core_en_mask`; the group's compute and fragment masks and counts (`sched`) | read already | panthor_fw.c:979 `panthor_fw_init_global_iface`; panthor_sched.c `panthor_group_create` (masks checked against it) |
| `TILER_PRESENT` | the group's tiler mask and count (`sched`) | read already | panthor_sched.c `panthor_group_create` |
| `AS_PRESENT` | the group's address space (`sched`, `nanopc_t6_submit` step 4) | read already | panthor_mmu.c:736-739 `panthor_vm_active`; panthor_sched.c `panthor_sched_init` (`hweight32(as_present & GENMASK(31, 1))`) |
| `L2_FEATURES` line size | the heap context stride (`heap::ctx_stride`) | read already | panthor_heap.c `panthor_heap_ctx_stride` |
| `CORE_FEATURES` (0x8) | not by the kernel side: `ident` reads it and the T6 OSAL hands it to the backend in `OSAL_QUERY_MALI_GPU` (`core_features`), whose frames are sized for the model it names with `GPU_ID` (`src/mali/model.rs`; its low byte is Mesa's variant, `src/panfrost/lib/kmod/panthor_kmod.c:162`) | not read | read into `gpu_info.core_features` and handed to user space (`DEV_QUERY`), as Mesa reads it |
| firmware interface version, `group_num`, `stream_num` | `csf`: bounds `MIN_CSGS..=MAX_CSGS`, `MIN_CS_PER_CSG..=MAX_CS_PER_CSG` | read already | panthor_fw.c:941 (version 0 refused), :953-954 `panthor_fw_init_ifaces`, :891-892 `panthor_init_csg_iface`; panthor_sched.c `panthor_sched_init` (the slot counts) |

Not used by the kernel side, as in panthor, where they only go to user space
(`DEV_QUERY`): `THREAD_*`, `TEXTURE_FEATURES`, `CSF_ID`, `TILER_FEATURES`,
`MEM_FEATURES` (panthor_hw.c:65-85 reads them all into `gpu_info`).
`GPU_FEATURES` is an arch-11 register (panthor_hw.c:87-88); arch 11 and later
are refused.

### Identity

`identify` takes the arch-10 products of panthor's table: 2 Mali-G710, 3
Mali-G510, 4 Mali-G310, 7 Mali-G610. The arch minor and revision and the
version fields do not decide (a G610 with another status is a G610). It
refuses by name: arch 11-13 with panthor's names (G715, G615, G720, G620,
G725, G925, G625; this kernel side was not written for them; where panthor
picks the name by the shader core count, the refusal names the family's
members), an arch below 10 as a
job-manager GPU (no CSF), anything else as panthor's "(Unknown Mali GPU)".
The T6's stop keeps its number (6), renamed `UnsupportedGpu`
(`YD_T6_GPU_STOP_UNSUPPORTED_GPU`); nothing is written to the GPU after it.

### Page tables

`Shape::from_mmu_features` takes VA and PA bits from 32 to 48: 48 is the most a
4 KiB-granule stage-1 walk without LPA2 takes, 32 the least that holds the MCU's
4 GiB VM. The walk has `ceil((va_bits - 12) / 9)` levels (io-pgtable's
`ARM_LPAE_S1` from `ias`): 48 and 40 bits four levels (a 40-bit root uses two
descriptors), 39 bits three levels from a level-1 root; `AS_TRANSCFG`'s input
size is `55 - va_bits` (0x4200_01c6 at 48, 0x4200_0406 at 39). The host test
`board::nanopc_t6_mmu::tests::a_narrower_va_builds_fewer_levels_and_says_so_in_transcfg`
shows it. Outside 32..48 Z2 stops with `Stop::Features` before any write.

### SoC and GPU

`gpu::mali::probe` is the GPU's own part of the power-on (identity, model, soft
reset, identity again, L2) and touches no SoC register; `gpu_up` runs it after
the RK3588's supply, clock, PD_GPU, take-over and QoS steps. A second SoC calls
its own power-on, then `probe`. Nothing else in `gpu::mali` touches a SoC
register.

## The G310 mock

`driver/rust/src/gpu/mali/mock.rs`, beside the G610's table (the T6's probe
line).

| Register | Value | Source |
|---|---|---|
| `GPU_ID` | 0xac740001: arch 10.12 r7, product 4, r0p0 status 1 | panthor's probe line on an i.MX 95, "mali-g310 id 0xac74 major 0x0 minor 0x0 status 0x1" (id is `GPU_ID >> 16`), in the review of "drm/panthor: Add i.MX95 support", dri-devel and linux-arm-kernel, February 2025 (<https://lists.freedesktop.org/archives/dri-devel/2025-February/493478.html> and its replies, <https://lists.infradead.org/pipermail/linux-arm-kernel/2025-February/1006422.html>). Arch 10.12 product 4 is Mesa 26.2.1's G310 (`src/panfrost/model/pan_model.c:91-95`, `PAN_PROD_ID(10, 12, 4)`) and panthor's (panthor_hw.c:25-26) |
| `L2_FEATURES` | 0x07110306 (64-byte line) | the same thread's boot log, "Features: L2:0x7110306 Tiler:0x809 Mem:0x1" |
| `TILER_FEATURES`, `MEM_FEATURES` | 0x809, 0x1 | the same line |
| `CORE_FEATURES` | 0 (variant 0, G310v1) | **assumed**: the variant (v1-v5) is unknown until the board is read |
| `MMU_FEATURES` | 0x2830 (48 VA, 40 PA) | **assumed** (the G610's) |
| `AS_PRESENT` | 0xff | **assumed** (the G610's) |
| `SHADER_PRESENT` | 0x3 | **assumed** (chosen to differ from the G610's) |
| `TILER_PRESENT`, `L2_PRESENT` | 1, 1 | **assumed** (`MEM_FEATURES` 0x1 says one L2 slice) |

The thread's two lines were read on 2026-11-20 as a search engine quotes them;
the archive pages themselves could not be fetched from the host that wrote
this. Their `MMU:`, `AS:` and `shader_present=` parts were not in the quoted
text. The log came from an NXP BSP firmware (`mali-imx-r50.2`), so a
mainline probe may read another version field. Mesa's G310 variants (v1-v5)
are told apart by `GPU_CORE_FEATURES & 0xff`
(`src/panfrost/lib/kmod/panthor_kmod.c:162`): v1-v3 have half the tile
buffer of v4-v5 and the G610 (`pan_model.c:91-99`), which sizes the frames'
tiles. `ident` reads the register and the backend takes the variant from it
(`src/mali/model.rs`, docs/notes/mali-g310-vs-g610.md section 5); which
variant the i.MX 95 has is not known until the board's register is read
(nothing public read for this page gives it). The mock's `CORE_FEATURES` is
0 (G310v1), assumed, and decides only the mock. The G610 mock's is 0 too:
Mesa 26.2.1 knows the RK3588's GPU as the G610, whose one row is variant 0
(`pan_model.c:88`, matched exactly, :139-149); its upper bits are assumed.
Every assumed value is in one place, `mock::G310_ASSUMED`, and
is replaced on the first day on the board by panthor's probe lines under the
NXP BSP or mainline (`Features: L2:.. Tiler:.. Mem:.. MMU:.. AS:..`,
`shader_present=.. l2_present=.. tiler_present=..`) or by our own
`gpu::mali::ident` dump (`CORE_FEATURES` 0x8, `MMU_FEATURES` 0x14, `AS_PRESENT` 0x18,
`SHADER_PRESENT` 0x100, `TILER_PRESENT` 0x110, `L2_PRESENT` 0x120); the
quoted `GPU_ID` and feature words are checked the same day.

The host tests that run on both mocks: `gpu::mali::tests::the_probe_runs_alike_on_the_g610_and_the_g310`
(identity, reset, L2; the same writes), `the_g310_reads_its_own_registers`,
`csf::tests::the_boot_on_the_g310_enables_its_own_cores` (the firmware
interface: the same writes, `core_en_mask` the G310's), and
`board::nanopc_t6_mmu::tests::as0_comes_up_on_the_g610_and_the_g310_from_their_mmu_features`.

## Still the G610's or the RK3588's

- `GPU_COHERENCY_PROTOCOL` is written "none" (`mali::COHERENCY_NONE` in
  `l2_power_on`): the RK3588's GPU node has no `dma-coherent`. panthor writes
  ACE-Lite on a coherent SoC (panthor_gpu.c:245 `panthor_gpu_coherency_set`); the
  i.MX 95's node decides it. Then `transcfg(.., coherent)`,
  `mair_to_memattr(.., coherent)` and the mapping caches follow.
- `MEMATTR_LIVE` (0x8F8F8F8F8C4C8F4C) is the word measured on the T6; bytes 0-3
  are `mair_to_memattr`'s, bytes 4-7 the T6's reading.
- The group VM's kernel VA range starts at 0x8000_0000_0000 (panthor's top half
  of a 48-bit VA with Mesa's 2^47 user VA, `heap.rs`); a GPU with fewer VA bits
  refuses those regions at the VM map (`PtRefusal::OutOfRange`). The range
  comes from the image's frame, not from the GPU.
- The arch timer's rate `RK3588_TIMER_HZ` is passed in (`timer_hz`), the
  caller's.
- The firmware the images embed is the RK3588's `arch10.8/mali_csffw.bin`; a
  G310 needs `arch10.12` (`fw_arch`), whose interface counts `csf` reads.
- Only the first L2 is powered (mask 1, as panthor_gpu.c:247) and
  `SHADER_PRESENT` goes whole into `core_en_mask` (as panthor_fw.c:979). With
  more than one bit in `L2_PRESENT` panthor only reports that it uses the first
  core group (panthor_gpu.c:229-242); neither the G610 nor (assumed) the G310
  has a second.
- The RK3588's supply, clock, PD_GPU and QoS (`board::nanopc_t6_gpu`,
  `soc::rk3588`) stay where they are; the i.MX 95's SCMI power and clocks are
  not written here.
