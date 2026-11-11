<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mp2_gpu

The STM32MP257F-DK's GPU powered from our image, and its identity read
(docs/notes/pre-study-stm32mp257f-dk.md section 6 item 2, P5; issues #7 and
#1281). It is `mp2_identify` (the banner, the hand-over record, OP-TEE's
watchdog armed at boot, PSCI SYSTEM_RESET at the end) with the GPU steps
added:

1. Power on (`yd_mp2_gpu_power_on`): OP-TEE opened (EXCHANGE_CAPABILITIES,
   the shared page registered, a session to the SCMI pseudo-TA with the
   login REE_KERNEL, the channel's handle), then SCMI PERFORMANCE domain 1
   (version, the level read, the same level set and read back), SCMI POWER
   domain 0 (version, on, read back), the RCC gate on and the reset pulsed
   in `RCC_GPUCFGR`, then `HI_IDLE_STATE` until idle; the session closed.
2. Identity (`yd_mp2_gpu_identify`): the identity registers and the
   hardware database row (`driver/rust/src/gpu/vivante/hw.rs`).
3. `HI_INTR_ENBL` (`yd_mp2_gpu_intr_check`): 0xffffffff written, read back,
   0 written again (no handler is connected).
4. Reset to idle (`yd_mp2_gpu_reset_idle`): the RCC reset asserted and
   released, then `HI_IDLE_STATE` until idle (at most 1000 reads).

PASS needs model 0x8000 and revision 0x6205 (Linux's, #1281), the
interrupt enable read back as written, and idle after the reset. Anything
else (a refused or failed SCMI call, an OP-TEE error, a timeout, a GPU
access the crate refuses) prints `verdict FAIL ...` and resets the board
at once.

The work is the yakogl-drivers crate's
(`driver/rust/src/soc/stm32mp2/gpu_ffi.rs` over `power.rs`, `rcc.rs` and
`firmware/optee`; C declarations in
`driver/zephyr/lib/board/stm32mp2/include/stm32mp2_gpu.h`), built with the
features `stm32mp2` and `vivante` by the board layer's
`CONFIG_YAKOGL_STM32MP2_GPU` (`prj.conf`; the archive `gc_fence` and
`gc_clear` link on this board, `driver/zephyr/CMakeLists.txt`). The host
tests (`cargo test -p yakogl-drivers stm32mp2`) pin every SMC, message frame
and SCMI byte against `tools/testdata/optee-scmi`, and the order of the RCC
and GPU accesses.

What it touches: the console, the system timer, SMCs to TF-A (PSCI) and to
OP-TEE (the watchdog, the SCMI pseudo-TA), `RCC_GPUCFGR` (0x44200000 +
0x83c), and the GPU's registers below 0x800 at 0x48280000. Nothing at or
above the GPU's 0x800 (the crate refuses such an access before the bus), no
clock rate, no PLL, no RIFSC. The risk is the OP-TEE panic of the brief
(the pre-study note's "The etnaviv panic and what our image avoids"): every
message is on the console before it is sent, so a panic leaves the last
request in the log.

## Build

```
west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_gpu \
  yakogl/tools/probes/stm32mp257f_dk/mp2_gpu
```

## What it prints

```
MP2:A EL2
...
gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0xbc000000, timeout 32 s, ...
*** Booting Zephyr OS build ... ***
mp2_gpu: build <git describe>
mp2_gpu: handed over at EL2, x0 0x..., sctlr 0x..., PSCI 0x10001
mp2_gpu: gpu 0x48280000 (0x800 bytes) at 0x..., rcc 0x44200000 at 0x..., shm pa 0x...
mp2_gpu: optee smc 0xb2000009 0x0 0x0 0x0 -> 0x0 0x... 0x... 0x...
mp2_gpu: optee msg > 0400000000000000...
mp2_gpu: optee smc 0x32000012 0x0 0x... 0x0 -> 0x0 0x0 0x0 0x0
mp2_gpu: optee msg < 0400000000000000...
...
mp2_gpu: scmi > 004c0400
mp2_gpu: optee msg > 01000000040000000...
mp2_gpu: optee smc 0x32000012 ...
mp2_gpu: optee msg < 01000000040000000...
mp2_gpu: scmi < 004c0400000000000000...
...
mp2_gpu: rcc gpucfgr 0x00000002
mp2_gpu: gpu idle 0x7fffffff after 1 reads
...
mp2_gpu: powered: level N (found N), rcc 0x00000002, idle 0x7fffffff after 1 reads
mp2_gpu: identity 0x...: GC8000 rev 0x6205 product 0x80003 customer 0x... eco 0 (built ...)
...
mp2_gpu: model=0x8000 rev=0x6205 product=0x80003 eco=0x0 customer=0x... date=0x... time=0x... features=0x... table=1 streams=16
mp2_gpu: gpu intr_enbl wrote 0xffffffff read 0xffffffff, then 0
mp2_gpu: rcc gpucfgr 0x00000003 (reset held)
mp2_gpu: rcc gpucfgr 0x00000002
mp2_gpu: gpu idle 0x7fffffff after 1 reads
mp2_gpu: reset: rcc 0x00000003 held, 0x00000002 released; idle 0x7fffffff after 1 reads
mp2_gpu: verdict PASS model=0x8000 rev=0x6205 product=0x80003 eco=0x0 customer=0x... intr=ok idle=ok
```

Not yet run on the board.
