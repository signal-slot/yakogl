/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Z1's record as the NanoPC-T6 probes print it (t6_z1.c): what
 * yd_t6_gpu_up read at the hand-over and on each step it reached. t6_gpu
 * prints it on every run; t6_mmu, t6_fw and t6_submit print it when Z1's
 * power-on stops, so a stop explains itself in the same record. Each line
 * starts with "<probe>: ".
 */
#ifndef T6_Z1_H
#define T6_Z1_H

#include <stdbool.h>
#include <stdint.h>

#include <yakogl_drivers.h>

/* The name of YD_T6_GPU_STOP_<stop>, "unknown" for another number. */
const char *t6_z1_stop_name(uint32_t stop);

/* The hand-over lines: PD_GPU, the CRU, SPI2 and the RK806 as Linux left
 * them (what on_handover sees, before the first PMU or GPU write). */
void t6_z1_print_handover(const char *probe, const struct yd_t6_gpu_up *h);

/* Z1's record after yd_t6_gpu_up returned: the hand-over lines first when
 * they were not printed yet (handover_printed false), then the clock set,
 * the supply enable and the lines of each step the run reached; for a run
 * that stopped nothing else (the probe prints its own FAIL line), for one
 * that did not the L2 line. */
void t6_z1_print_record(const char *probe, const struct yd_t6_gpu_up *o, bool handover_printed);

/* A Z1 stop in a probe that did not print the hand-over: the whole record,
 * then "<probe>: Z1 stopped: stop N (name), refusal R, detail 0x...". */
void t6_z1_print_stop(const char *probe, const struct yd_t6_gpu_up *o);

#endif /* T6_Z1_H */
