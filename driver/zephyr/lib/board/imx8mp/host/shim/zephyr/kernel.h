/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The few Zephyr kernel calls board_imx8mp.c uses, on POSIX threads, so that
 * the host model (../latch_model.c) compiles the board file unchanged. One
 * tick is one microsecond of CLOCK_MONOTONIC.
 */
#ifndef LATCH_MODEL_ZEPHYR_KERNEL_H
#define LATCH_MODEL_ZEPHYR_KERNEL_H

#include <pthread.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
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

static inline int64_t model_now_us(void)
{
	struct timespec ts;

	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (int64_t)ts.tv_sec * 1000000 + ts.tv_nsec / 1000;
}

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
	pthread_mutex_t m;
};
#define K_MUTEX_DEFINE(name) struct k_mutex name = { PTHREAD_MUTEX_INITIALIZER }
static inline int k_mutex_lock(struct k_mutex *mu, k_timeout_t t)
{
	ARG_UNUSED(t);
	return pthread_mutex_lock(&mu->m);
}
static inline int k_mutex_unlock(struct k_mutex *mu)
{
	return pthread_mutex_unlock(&mu->m);
}

struct k_condvar {
	pthread_cond_t c;
};
#define K_CONDVAR_DEFINE(name) struct k_condvar name = { PTHREAD_COND_INITIALIZER }
/* 0 when signalled, -EAGAIN at the deadline (as Zephyr) */
static inline int k_condvar_wait(struct k_condvar *cv, struct k_mutex *mu, k_timeout_t t)
{
	struct timespec ts;
	int64_t left;

	if (t.abs_us < 0) {
		return pthread_cond_wait(&cv->c, &mu->m);
	}
	left = t.abs_us - model_now_us();
	if (left <= 0) {
		return -EAGAIN;
	}
	/* the static condvar runs on CLOCK_REALTIME: carry the time left over */
	clock_gettime(CLOCK_REALTIME, &ts);
	ts.tv_sec += left / 1000000;
	ts.tv_nsec += (left % 1000000) * 1000;
	if (ts.tv_nsec >= 1000000000) {
		ts.tv_sec++;
		ts.tv_nsec -= 1000000000;
	}
	return pthread_cond_timedwait(&cv->c, &mu->m, &ts) == 0 ? 0 : -EAGAIN;
}
static inline int k_condvar_broadcast(struct k_condvar *cv)
{
	return pthread_cond_broadcast(&cv->c);
}

struct k_sem {
	pthread_mutex_t m;
	pthread_cond_t c;
	unsigned int count, limit;
};
#define K_SEM_DEFINE(name, init, lim)                                                              \
	struct k_sem name = { PTHREAD_MUTEX_INITIALIZER, PTHREAD_COND_INITIALIZER, (init), (lim) }
static inline int k_sem_take(struct k_sem *s, k_timeout_t t)
{
	ARG_UNUSED(t);
	pthread_mutex_lock(&s->m);
	while (s->count == 0u) {
		pthread_cond_wait(&s->c, &s->m);
	}
	s->count--;
	pthread_mutex_unlock(&s->m);
	return 0;
}
static inline void k_sem_give(struct k_sem *s)
{
	pthread_mutex_lock(&s->m);
	if (s->count < s->limit) {
		s->count++;
	}
	pthread_cond_signal(&s->c);
	pthread_mutex_unlock(&s->m);
}

struct k_thread {
	pthread_t t;
};
typedef struct k_thread *k_tid_t;
typedef void (*k_thread_entry_t)(void *, void *, void *);
#define K_THREAD_STACK_DEFINE(name, size) char name[size]
#define K_THREAD_STACK_SIZEOF(s) sizeof(s)
#define K_LOWEST_APPLICATION_THREAD_PRIO 14
struct model_thread_arg {
	k_thread_entry_t fn;
	void *p1, *p2, *p3;
};
static inline void *model_thread_main(void *a)
{
	struct model_thread_arg arg = *(struct model_thread_arg *)a;

	free(a);
	arg.fn(arg.p1, arg.p2, arg.p3);
	return NULL;
}
static inline k_tid_t k_thread_create(struct k_thread *th, char *stack, size_t size,
				      k_thread_entry_t fn, void *p1, void *p2, void *p3, int prio,
				      uint32_t options, k_timeout_t delay)
{
	struct model_thread_arg *a = malloc(sizeof(*a));

	ARG_UNUSED(stack);
	ARG_UNUSED(size);
	ARG_UNUSED(prio);
	ARG_UNUSED(options);
	ARG_UNUSED(delay);
	*a = (struct model_thread_arg){ fn, p1, p2, p3 };
	pthread_create(&th->t, NULL, model_thread_main, a);
	pthread_detach(th->t);
	return th;
}

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
