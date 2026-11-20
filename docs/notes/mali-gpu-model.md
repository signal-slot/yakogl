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
behaviour and cited by file and function (MIT option of its licence, the design
page's decision 1). The panthor source could not be fetched on the host that
wrote this change (no network): the functions named below are cited from the
earlier reading the code's comments already carry; their line numbers are to
be checked against v6.18 when the source is at hand.

## What is read from the GPU

| Value | Where it is used | Before | panthor |
|---|---|---|---|
| `GPU_ID` arch major, product major | `gpu::mali::identify` (`probe`, `board::nanopc_t6_gpu::gpu_up`): the model, or stop 6 `UnsupportedGpu` | `== 0xa8670005` | panthor_hw.c `get_gpu_model_name` (`GPU_PROD_ID_MAKE(GPU_ARCH_MAJOR, GPU_PROD_MAJOR)`) |
| `GPU_ID` arch major, minor | `gpu::mali::fw_arch`: the firmware's directory, `arm/mali/arch<maj>.<min>/` | `arch10.8` in comments | panthor_fw.c `panthor_fw_load` |
| `MMU_FEATURES` VA bits 7:0, PA bits 15:8 | `pgtable::Shape`: levels, root size, VA and PA range checks of `Tables`; `mmu::transcfg`'s input size; `nanopc_t6_mmu::as0_up` (AS0), `nanopc_t6_submit` (AS0 reopened, AS1) | `VA_BITS` 48, `PA_BITS` 40, 4 levels; the check refused all but 48/≥40 | panthor_mmu.c `panthor_vm_create` (`ias`/`oas` to io-pgtable), `panthor_vm_active` (`AS_TRANSCFG_INA_BITS(55 - va_bits)`) |
| `L2_PRESENT` | `l2_power_on`: the mask powered (the first L2), `NoL2` when 0 | read already | panthor_gpu.c `panthor_gpu_l2_power_on` |
| `SHADER_PRESENT` | `csf::global_init`'s `core_en_mask`; the group's compute and fragment masks and counts (`sched`) | read already | panthor_fw.c `panthor_fw_init_global_iface`; panthor_sched.c group creation |
| `TILER_PRESENT` | the group's tiler mask and count (`sched`) | read already | panthor_sched.c |
| `AS_PRESENT` | the group's address space (`sched`, `nanopc_t6_submit` step 4) | read already | panthor_mmu.c (slot allocation) |
| `L2_FEATURES` line size | the heap context stride (`heap::ctx_stride`) | read already | panthor_heap.c `panthor_heap_ctx_stride` |
| firmware `GLB_VERSION`, `group_num`, `stream_num` | `csf`: bounds `MIN_CSGS..=MAX_CSGS`, `MIN_CS_PER_CSG..=MAX_CS_PER_CSG` | read already | panthor_fw.c `panthor_init_csg_iface`, `panthor_fw_init_ifaces` |

Not used by the kernel side, as in panthor, where they only go to user space
(`DEV_QUERY`): `THREAD_*`, `TEXTURE_FEATURES`, `CSF_ID`, `TILER_FEATURES`,
`MEM_FEATURES`. `GPU_FEATURES` is an arch-11 register; arch 11 and later are
refused.

### Identity

`identify` takes the arch-10 products of panthor's table: 2 Mali-G710, 3
Mali-G510, 4 Mali-G310, 7 Mali-G610. The arch minor and revision and the
version fields do not decide (a G610 with another status is a G610). It
refuses by name: arch 11-13 with panthor's names (G715, G615, G720, G620,
G725, G625; this kernel side was not written for them), an arch below 10 as a
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
| `GPU_ID` 31:28, 27:24, 19:16 | arch 10, arch minor 12, product 4 | Mesa 26.2.1 `src/panfrost/model/pan_model.c:91-100` (`PAN_PROD_ID(10, 12, 4)`, "G310v1".."G310v5") |
| `GPU_ID` 23:20, 15:0 | 0 (so `GPU_ID` 0xac040000) | **assumed** |
| `L2_FEATURES` | 0x07120306 (64-byte line) | **assumed** (the G610's) |
| `TILER_FEATURES`, `MEM_FEATURES` | 0x809, 0x301 | **assumed** (the G610's) |
| `MMU_FEATURES` | 0x2830 (48 VA, 40 PA) | **assumed** (the G610's) |
| `AS_PRESENT` | 0xff | **assumed** |
| `SHADER_PRESENT` | 0x3 | **assumed** (chosen to differ from the G610's) |
| `TILER_PRESENT`, `L2_PRESENT` | 1, 1 | **assumed** |

Mesa's G310 variants (v1-v5) are told apart by `GPU_CORE_FEATURES & 0xff`
(`src/panfrost/lib/kmod/panthor_kmod.c:162`); the kernel side does not read
that register. No i.MX 95 panthor probe line from a mailing list or a bug report
could be read from the host that wrote this (no web access): every assumed value
is in one place, `mock::G310_ASSUMED`, and is replaced on the first day on the
board by panthor's three probe lines under the NXP BSP or mainline (`Mali-G310
id ...`, `Features: L2:.. Tiler:.. Mem:.. MMU:.. AS:..`, `shader_present=..
l2_present=.. tiler_present=..`) or by our own `gpu::mali::ident` dump.

The host tests that run on both mocks: `gpu::mali::tests::the_probe_runs_alike_on_the_g610_and_the_g310`
(identity, reset, L2; the same writes), `the_g310_reads_its_own_registers`,
`csf::tests::the_boot_on_the_g310_enables_its_own_cores` (the firmware
interface: the same writes, `core_en_mask` the G310's), and
`board::nanopc_t6_mmu::tests::as0_comes_up_on_the_g610_and_the_g310_from_their_mmu_features`.

## Still the G610's or the RK3588's

- `GPU_COHERENCY_PROTOCOL` is written "none" (`mali::COHERENCY_NONE` in
  `l2_power_on`): the RK3588's GPU node has no `dma-coherent`. panthor writes
  ACE-Lite on a coherent SoC (panthor_gpu.c `panthor_gpu_coherency_set`); the
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
- Only the first core group is powered and `SHADER_PRESENT` is used whole:
  panthor does the same mask arithmetic only when `L2_PRESENT` has more than one
  bit, which neither the G610 nor (assumed) the G310 has.
- The RK3588's supply, clock, PD_GPU and QoS (`board::nanopc_t6_gpu`,
  `soc::rk3588`) stay where they are; the i.MX 95's SCMI power and clocks are
  not written here.
