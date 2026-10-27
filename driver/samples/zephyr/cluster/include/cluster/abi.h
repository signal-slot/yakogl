/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * SafeUI Application ABI — shared between the A53 Supervisor, the app SDK
 * runtime and the applications themselves.  Everything in this header is
 * plain data: no kernel objects, no pointers into privileged memory.
 *
 * ABI versioning: CLUSTER_ABI_MAJOR must match exactly between the platform
 * and the app package; CLUSTER_ABI_MINOR of the platform must be >= the one
 * the app was built against.
 */
#ifndef CLUSTER_ABI_H_
#define CLUSTER_ABI_H_

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CLUSTER_ABI_MAJOR 1
#define CLUSTER_ABI_MINOR 0

#define CLUSTER_APP_ID_MAX 32
#define CLUSTER_ENTRY_SYMBOL "cluster_app_main"

/* Capabilities an app may request in its manifest and the Supervisor may grant. */
#define CLUSTER_CAP_DISPLAY  (1ULL << 0) /* surface get / present */
#define CLUSTER_CAP_INPUT    (1ULL << 1) /* touch input events */
#define CLUSTER_CAP_SENSOR   (1ULL << 2) /* read-only sensor data published by R5F */
#define CLUSTER_CAP_STORAGE  (1ULL << 3) /* persistent per-app key/value store */
#define CLUSTER_CAP_TIME     (1ULL << 4) /* monotonic clock and timers */
#define CLUSTER_CAP_LOG      (1ULL << 5) /* rate-limited log output */
#define CLUSTER_CAP_GPU      (1ULL << 6) /* GPU context and command submission */
#define CLUSTER_CAP_THREADS  (1ULL << 7) /* create additional app threads */
#define CLUSTER_CAP_ALL      ((1ULL << 8) - 1)

/* Pixel formats of app surfaces. */
#define CLUSTER_PIXFMT_RGB565 1

struct cluster_surface_info {
	void *pixels;          /* base of the app-owned surface, mapped RW in the app domain */
	uint32_t width;
	uint32_t height;
	uint32_t stride_bytes;
	uint32_t format;       /* CLUSTER_PIXFMT_* */
	uint32_t size_bytes;
};

/* Input events */
#define CLUSTER_INPUT_TOUCH_DOWN 1
#define CLUSTER_INPUT_TOUCH_MOVE 2
#define CLUSTER_INPUT_TOUCH_UP   3
#define CLUSTER_INPUT_KEY        4

struct cluster_input_event {
	uint32_t type;
	int32_t x;
	int32_t y;
	uint32_t code;
	uint64_t timestamp_ms;
};

/* Sensor ids published by the R5F Platform Firmware (read-only for apps). */
enum cluster_sensor_id {
	/* meter cluster demo */
	CLUSTER_SENSOR_SPEED_KMH_X10 = 0,
	CLUSTER_SENSOR_GEAR,             /* -1 R, 0 N, 1.. */
	CLUSTER_SENSOR_FUEL_ML,
	CLUSTER_SENSOR_FUEL_TANK_ML,
	CLUSTER_SENSOR_ODOMETER_M,
	CLUSTER_SENSOR_TRIP_SINCE_REFUEL_M,
	CLUSTER_SENSOR_INDICATOR,        /* bit0 left, bit1 right */
	CLUSTER_SENSOR_ENGINE_RPM,
	/* medical monitor demo */
	CLUSTER_SENSOR_HEART_RATE_BPM,
	CLUSTER_SENSOR_SPO2_PCT,
	CLUSTER_SENSOR_RESP_RATE,
	CLUSTER_SENSOR_LEAD_STATUS,      /* bit per electrode: 1 = attached */
	CLUSTER_SENSOR_COUNT
};

#define CLUSTER_SENSOR_VALID  (1U << 0)
#define CLUSTER_SENSOR_STALE  (1U << 1)

struct cluster_sensor_value {
	int32_t value;
	uint32_t flags;
	uint64_t timestamp_ms;
} __attribute__((packed));

/* ECG sample stream: blocks of samples for up to 3 leads. */
#define CLUSTER_ECG_LEADS 3
#define CLUSTER_ECG_BLOCK_SAMPLES 32

struct cluster_ecg_block {
	uint32_t seq;
	uint32_t lead_mask;    /* bit n set => samples[n][] valid */
	uint32_t sample_rate_hz;
	int16_t samples[CLUSTER_ECG_LEADS][CLUSTER_ECG_BLOCK_SAMPLES];
};

/* Warnings owned and displayed by the R5F side; mirrored to apps for decoration only. */
#define CLUSTER_WARN_FUEL_LOW        (1U << 0)
#define CLUSTER_WARN_FUEL_EMPTY      (1U << 1)
#define CLUSTER_WARN_ENGINE          (1U << 2)
#define CLUSTER_WARN_LEAD_OFF        (1U << 8)
#define CLUSTER_WARN_HR_LIMIT        (1U << 9)
#define CLUSTER_WARN_SPO2_LOW        (1U << 10)
#define CLUSTER_WARN_SIGNAL_INVALID  (1U << 11)
#define CLUSTER_WARN_APP_DOWN        (1U << 16) /* rich UI app not running */

/* Events an app can wait for. */
#define CLUSTER_EV_VSYNC     (1U << 0)
#define CLUSTER_EV_INPUT     (1U << 1)
#define CLUSTER_EV_SENSOR    (1U << 2)
#define CLUSTER_EV_ECG       (1U << 3)
#define CLUSTER_EV_TERMINATE (1U << 31) /* Supervisor asks the app to exit */

/* GPU service: a small, validated command stream.  On the target this is
 * translated by the privileged GPU Service; on QEMU it is executed by a CPU
 * reference implementation of the same command set.
 */
#define CLUSTER_GPU_OP_CLEAR      1  /* a0 = rgb565 */
#define CLUSTER_GPU_OP_FILL_RECT  2  /* a0 = x, a1 = y, a2 = w, a3 = h, a4 = rgb565 */
#define CLUSTER_GPU_OP_LINE       3  /* a0 = x0, a1 = y0, a2 = x1, a3 = y1, a4 = rgb565 */
#define CLUSTER_GPU_OP_GRADIENT_V 4  /* a0 = y, a1 = h, a2 = rgb565 top, a3 = rgb565 bottom */
#define CLUSTER_GPU_OP_ARC        5  /* a0 = cx, a1 = cy, a2 = r, a3 = start deg, a4 = end deg, a5 = rgb565 */

struct cluster_gpu_cmd {
	uint32_t op;
	int32_t a[6];
};

#define CLUSTER_GPU_MAX_CMDS 512

/* Context passed to the application entry point.  Lives in app-readable memory. */
struct cluster_app_context {
	uint32_t abi_major;
	uint32_t abi_minor;
	char app_id[CLUSTER_APP_ID_MAX];
	uint32_t version[3];
	uint64_t capabilities;      /* granted subset of the manifest request */
	void *heap_base;            /* app-private heap region (RW in app domain) */
	size_t heap_size;
	uint32_t restart_count;     /* how many times this app was restarted since boot */
	uint32_t last_fault_reason; /* 0 if the previous instance exited cleanly */
};

typedef int (*cluster_app_main_t)(const struct cluster_app_context *ctx);

#ifdef __cplusplus
}
#endif

#endif /* CLUSTER_ABI_H_ */
