/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Message protocol between the R5F Platform Firmware and the A53 Supervisor.
 *
 * Transport is out of scope of this header: on the SoM it is a mailbox
 * interrupt plus a shared-memory ring (see platform/fw and supervisor
 * link_ipm.c); on QEMU both ends live in one image and messages are
 * delivered through function calls (link_sim.c).  Messages are fixed-layout,
 * little-endian and self-describing so either side can be replaced.
 */
#ifndef CLUSTER_IPC_H_
#define CLUSTER_IPC_H_

#include <stdint.h>
#include <cluster/abi.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CLUSTER_IPC_VERSION 1

enum cluster_ipc_type {
	/* A53 -> R5F */
	CLUSTER_IPC_HEARTBEAT = 1,
	CLUSTER_IPC_APP_STATUS = 2,
	CLUSTER_IPC_FRAME = 3,      /* a finished application frame in DDR */
	CLUSTER_IPC_INPUTS = 4,     /* operator override of the R5F's physical inputs
				    * (struct plat_inputs; the shell's `plat` command) */
	/* R5F -> A53 */
	CLUSTER_IPC_SNAPSHOT = 16,
	CLUSTER_IPC_ECG_BLOCK = 17,
	CLUSTER_IPC_TOUCH = 18,     /* touch event from the controller owned by the R5F */
	CLUSTER_IPC_VSYNC = 19,     /* DSS vertical sync: the A53 compositor paces on it */
};

/* Application frame hand-over (method 2 of docs/display-sharing.md): the A53
 * renders into DDR and tells the R5F where the finished frame is; the R5F
 * points the DSS application plane at it.  Format codes are DSS codes.
 */
struct cluster_frame {
	uint64_t pa;
	uint32_t width;
	uint32_t height;
	uint32_t stride_bytes;
	uint32_t format;          /* 0x3 RGB565, 0x7 ARGB8888 */
	uint32_t seq;
	uint8_t app_alive;        /* 0 => hide the plane */
	uint8_t pad[3];
} __attribute__((packed));

struct cluster_vsync {
	uint32_t count;
	uint32_t app_seq_shown;   /* seq of the A53 frame currently on the plane */
} __attribute__((packed));

struct cluster_touch {
	uint32_t type;            /* CLUSTER_INPUT_* */
	int32_t x;
	int32_t y;
} __attribute__((packed));

struct cluster_ipc_hdr {
	uint16_t type;
	uint16_t len;      /* payload length in bytes */
	uint32_t seq;
} __attribute__((packed));

enum cluster_supervisor_state {
	CLUSTER_SUP_BOOTING = 0,
	CLUSTER_SUP_RUNNING = 1,
	CLUSTER_SUP_DEGRADED = 2,   /* at least one app in crash loop */
};

struct cluster_heartbeat {
	uint32_t counter;
	uint32_t supervisor_state;
	uint64_t uptime_ms;
} __attribute__((packed));

enum cluster_app_state {
	CLUSTER_APP_EMPTY = 0,
	CLUSTER_APP_LOADED,
	CLUSTER_APP_RUNNING,
	CLUSTER_APP_FAULTED,
	CLUSTER_APP_RECLAIMING,
	CLUSTER_APP_WAIT_RESTART,
	CLUSTER_APP_CRASH_LOOP,
	CLUSTER_APP_STOPPED,        /* stopped by operator or policy, no restart */
	CLUSTER_APP_REJECTED,       /* package failed verification */
};

struct cluster_app_status {
	char app_id[CLUSTER_APP_ID_MAX];
	uint8_t slot;
	uint8_t state;             /* enum cluster_app_state */
	uint16_t fault_count;
	uint32_t restart_count;
	uint32_t last_fault_reason;
	uint32_t frames_presented; /* since last (re)start */
} __attribute__((packed));

/* Sensor snapshot published by the platform side at a fixed rate. */
struct cluster_snapshot {
	uint32_t gen;              /* increments every publish */
	uint32_t input_gen;        /* increments only when a physical input changes state */
	uint32_t warnings;         /* CLUSTER_WARN_* as evaluated by the R5F */
	uint32_t safety_frame;     /* R5F safety display frame counter */
	uint8_t safety_bg_opaque;  /* 1 => R5F hides the A53 layer */
	uint8_t a53_alive;
	uint8_t pad[2];
	struct cluster_sensor_value sensors[CLUSTER_SENSOR_COUNT];
} __attribute__((packed));

#ifdef __cplusplus
}
#endif

#endif /* CLUSTER_IPC_H_ */
