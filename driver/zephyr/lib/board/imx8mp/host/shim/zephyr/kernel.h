/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The few Zephyr kernel calls board_imx8mp.c uses, so that the host model
 * (../latch_model.c) compiles the board file unchanged. Threads are POSIX
 * threads that run one at a time on the simulated clock of ../sim_kernel.c.
 */
#ifndef LATCH_MODEL_ZEPHYR_KERNEL_H
#define LATCH_MODEL_ZEPHYR_KERNEL_H

#include <pthread.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>

/* zephyr/toolchain.h, zephyr/sys/util.h */
#define FUNC_NORETURN __attribute__((noreturn))
#define CODE_UNREACHABLE __builtin_unreachable()
#define ARG_UNUSED(x) ((void)(x))
#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))
#define __weak __attribute__((weak))
/* IS_ENABLED as Zephyr spells it: 1 for a macro defined as 1, else 0 */
#define Z_IS_ENABLED_1(x) Z_IS_ENABLED_2(_XXXX##x)
#define _XXXX1 _YYYY,
#define Z_IS_ENABLED_2(one_or_two_args) Z_IS_ENABLED_3(one_or_two_args 1, 0)
#define Z_IS_ENABLED_3(ignore_this, val, ...) val
#define IS_ENABLED(config_macro) Z_IS_ENABLED_1(config_macro)

/* The kernel calls below are defined in ../sim_kernel.c, on a simulated
 * clock: model_now_us() is the simulated time in us, and one tick is one us
 * of it. model_kernel_init() makes the caller the first model thread. */
struct model_thread;
void model_kernel_init(void);
int64_t model_now_us(void);
void model_sleep_until_us(int64_t t);

/* k_timeout_t: K_FOREVER, K_NO_WAIT or an absolute deadline in ticks (us) */
typedef struct {
	int64_t abs_us; /* -1 for ever */
} k_timeout_t;
#define K_FOREVER ((k_timeout_t){ .abs_us = -1 })
#define K_NO_WAIT ((k_timeout_t){ .abs_us = 0 })
#define K_TIMEOUT_ABS_TICKS(t) ((k_timeout_t){ .abs_us = (t) })
static inline int64_t k_uptime_ticks(void)
{
	return model_now_us();
}
static inline uint64_t k_us_to_ticks_ceil64(uint64_t us)
{
	return us;
}

struct k_mutex {
	struct model_thread *owner;
};
#define K_MUTEX_DEFINE(name) struct k_mutex name = { NULL }
int k_mutex_lock(struct k_mutex *mu, k_timeout_t t);
int k_mutex_unlock(struct k_mutex *mu);

struct k_condvar {
	int unused; /* the waiters are found by the condvar's address */
};
#define K_CONDVAR_DEFINE(name) struct k_condvar name = { 0 }
/* 0 when signalled, -EAGAIN at the deadline (as Zephyr) */
int k_condvar_wait(struct k_condvar *cv, struct k_mutex *mu, k_timeout_t t);
int k_condvar_broadcast(struct k_condvar *cv);

struct k_sem {
	unsigned int count, limit;
};
#define K_SEM_DEFINE(name, init, lim) struct k_sem name = { (init), (lim) }
int k_sem_take(struct k_sem *s, k_timeout_t t);
void k_sem_give(struct k_sem *s);

struct k_thread {
	pthread_t t;
};
typedef struct k_thread *k_tid_t;
typedef void (*k_thread_entry_t)(void *, void *, void *);
#define K_THREAD_STACK_DEFINE(name, size) char name[size]
#define K_THREAD_STACK_SIZEOF(s) sizeof(s)
#define K_LOWEST_APPLICATION_THREAD_PRIO 14
k_tid_t k_thread_create(struct k_thread *th, char *stack, size_t size, k_thread_entry_t fn,
			void *p1, void *p2, void *p3, int prio, uint32_t options,
			k_timeout_t delay);

#define printk printf

/* zephyr/device.h, zephyr/devicetree.h */
struct device {
	int unused;
};
extern const struct device model_gpu_device;
#define DEVICE_DT_GET_ONE(compat) (&model_gpu_device)
static inline bool device_is_ready(const struct device *d)
{
	return d != NULL;
}

/* zephyr/sys/reboot.h */
#define SYS_REBOOT_COLD 1
static inline void sys_reboot(int type)
{
	exit(type);
}

#endif
