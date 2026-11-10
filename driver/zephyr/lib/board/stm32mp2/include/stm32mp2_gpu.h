/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * stm32mp2_gpu.h -- the STM32MP257F's GPU power, identity and reset, from the
 * yakogl-drivers crate (driver/rust/src/soc/stm32mp2/gpu_ffi.rs; features
 * stm32mp2 and vivante). Plan #322, issues #7 and #1281.
 *
 * Every SMC, OP-TEE message frame and SCMI request and reply is handed to
 * the log callback as one line ("optee smc ...", "optee msg > ...",
 * "scmi > ...", ...), the request before it is sent. The GPU is only
 * reached below offset 0x800: an access at or above it is refused without
 * touching the bus (YD_MP2_GPU_ERR_WINDOW).
 *
 * Return values: 0, or a negative errno (Zephyr's numbering) with the reason
 * in the result's `err`.
 */
#ifndef YAKOGL_STM32MP2_GPU_H
#define YAKOGL_STM32MP2_GPU_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The memory shared with OP-TEE: three 4 KiB pages, 4 KiB aligned, normal
 * cached memory, physically contiguous. */
#define YD_MP2_GPU_SHM_SIZE  12288u
#define YD_MP2_GPU_SHM_ALIGN 4096u

/* One log line, `len` bytes, not NUL-terminated. */
typedef void (*yd_mp2_gpu_log_fn)(void *ctx, const char *line, size_t len);

struct yd_mp2_gpu {
	uintptr_t rcc_base;    /* the RCC, mapped (RCC_GPUCFGR at +0x83c) */
	uintptr_t gpu_base;    /* the GPU's registers, mapped, 0x800 bytes */
	void *shm;             /* YD_MP2_GPU_SHM_SIZE bytes, see above */
	uint64_t shm_pa;       /* its physical address */
	yd_mp2_gpu_log_fn log; /* may be NULL */
	void *log_ctx;
	uint32_t idle_reads;   /* bound of every idle poll; 0: 1000 */
	uint32_t channel;      /* the SCMI channel, 0 on the DK */
};

/* err.kind */
#define YD_MP2_GPU_ERR_NONE         0u
#define YD_MP2_GPU_ERR_OPTEE        1u  /* step 1 open, 2 close; a: the code */
#define YD_MP2_GPU_ERR_TRANSPORT    2u  /* a: the transport's code */
#define YD_MP2_GPU_ERR_REFUSED      3u  /* a: the SCMI status (negative) */
#define YD_MP2_GPU_ERR_ECHO         4u  /* a: header sent, b: received */
#define YD_MP2_GPU_ERR_LENGTH       5u  /* a: the reply's length */
#define YD_MP2_GPU_ERR_VERSION      6u  /* a: the version reported */
#define YD_MP2_GPU_ERR_READBACK     7u  /* a: wanted, b: read (identify: a the errno) */
#define YD_MP2_GPU_ERR_RCC          8u  /* a: RCC_GPUCFGR as read */
#define YD_MP2_GPU_ERR_IDLE_TIMEOUT 9u  /* a: reads made, b: last HI_IDLE_STATE */
#define YD_MP2_GPU_ERR_WINDOW       10u /* a: the GPU offset refused */

/* err.step for the SCMI kinds */
#define YD_MP2_GPU_STEP_PERF_VERSION     1u
#define YD_MP2_GPU_STEP_PERF_LEVEL_GET   2u
#define YD_MP2_GPU_STEP_PERF_LEVEL_SET   3u
#define YD_MP2_GPU_STEP_PERF_LEVEL_CHECK 4u
#define YD_MP2_GPU_STEP_POWER_VERSION    5u
#define YD_MP2_GPU_STEP_POWER_ON         6u
#define YD_MP2_GPU_STEP_POWER_ON_CHECK   7u

struct yd_mp2_gpu_err {
	uint32_t kind;
	uint32_t step;
	uint32_t a;
	uint32_t b;
};

struct yd_mp2_gpu_up {
	uint32_t level_before; /* the performance level found */
	uint32_t level;        /* the level set and read back */
	uint32_t rcc;          /* RCC_GPUCFGR after the reset's release */
	uint32_t idle;         /* HI_IDLE_STATE when idle */
	uint32_t idle_reads;
	struct yd_mp2_gpu_err err;
};

struct yd_mp2_gpu_id {
	uint32_t model;
	uint32_t revision;
	uint32_t product_id;
	uint32_t eco_id;
	uint32_t customer_id;
	uint32_t date;
	uint32_t time;
	uint32_t features;
	uint32_t from_table;   /* 1: a row of the hardware database matched */
	uint32_t stream_count; /* the row's, 0 without one */
	struct yd_mp2_gpu_err err;
};

struct yd_mp2_gpu_reset {
	uint32_t rcc_held; /* RCC_GPUCFGR with the reset asserted */
	uint32_t rcc;      /* and after its release */
	uint32_t idle;
	uint32_t idle_reads;
	struct yd_mp2_gpu_err err;
};

_Static_assert(sizeof(struct yd_mp2_gpu) == 56, "struct yd_mp2_gpu is the crate's YdMp2Gpu");
_Static_assert(sizeof(struct yd_mp2_gpu_up) == 36, "struct yd_mp2_gpu_up is the crate's");
_Static_assert(sizeof(struct yd_mp2_gpu_id) == 56, "struct yd_mp2_gpu_id is the crate's");
_Static_assert(sizeof(struct yd_mp2_gpu_reset) == 32, "struct yd_mp2_gpu_reset is the crate's");

/* OP-TEE opened (capabilities, the shared page registered, a session to the
 * SCMI pseudo-TA, the channel), then: PERFORMANCE domain 1 version, level
 * read, the same level set and read back; POWER domain 0 version, on, read
 * back; the RCC gate on, reset asserted and released, read back;
 * HI_IDLE_STATE until idle. The session is closed after, also on a failure.
 * A refused or failed SCMI step stops at once: no RCC write follows it.
 * -EIO (err says why), -ETIMEDOUT (not idle), -EFAULT (window), -EINVAL. */
int32_t yd_mp2_gpu_power_on(const struct yd_mp2_gpu *g, struct yd_mp2_gpu_up *out);

/* The identity registers and the hardware database row. -ENODEV when the
 * GPU does not answer (model 0, all ones, or the absent pattern). */
int32_t yd_mp2_gpu_identify(const struct yd_mp2_gpu *g, struct yd_mp2_gpu_id *out);

/* HI_INTR_ENBL: `value` written, read back into *readback, then 0 written
 * (no handler is connected). 0 when the read back equals `value`, -EIO. */
int32_t yd_mp2_gpu_intr_check(const struct yd_mp2_gpu *g, uint32_t value, uint32_t *readback);

/* The RCC reset asserted, read, released, read; then HI_IDLE_STATE until
 * idle (every module but AXI_LP). -EIO (RCC_GPUCFGR not as written),
 * -ETIMEDOUT (not idle). */
int32_t yd_mp2_gpu_reset_idle(const struct yd_mp2_gpu *g, struct yd_mp2_gpu_reset *out);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_STM32MP2_GPU_H */
