/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Inside the noinit output section (CMakeLists.txt, zephyr_linker_sources
 * NOINIT): room for one copy of the initialised RAM data, exactly
 * [__data_region_start, __data_region_end) -- the range Zephyr's XIP
 * arch_data_copy() would restore (arch/common/xip.c). src/main.c takes the
 * copy on the first entry and puts it back on every later one (an entry
 * after a SYSTEM_SUSPEND, a53_power.h). Not cleared at boot (noinit), and
 * the A53 image never maps the loader's RAM except the stamp page.
 */
. = ALIGN(8);
bs2_data_snapshot = .;
. += __data_region_end - __data_region_start;
bs2_data_snapshot_end = .;
