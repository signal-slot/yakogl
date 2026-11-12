/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Zephyr calls gc_render.c uses, on a simulated clock, so that the host
 * model (../../wait_model.c) compiles the render service unchanged and drives
 * gc_render_wait() through scripted interrupts. One thread, no real time: a
 * call that blocks (k_sem_take, k_sleep) moves the clock to the first of its
 * deadline and the next scripted interrupt, runs the interrupt there, and
 * returns as Zephyr would. The clock counts microseconds; K_MSEC(1) is 1000
 * of them (Zephyr's 1.0 to 1.1 ms at 10 000 ticks a second, without the
 * rounding).
 */
#ifndef WAIT_MODEL_ZEPHYR_KERNEL_H
#define WAIT_MODEL_ZEPHYR_KERNEL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <errno.h>

/* zephyr/toolchain.h, zephyr/sys/util.h */
#define ARG_UNUSED(x) ((void)(x))
#define __weak __attribute__((weak))
#define BIT(n) (1UL << (n))
#define ROUND_UP(x, a) ((((x) + ((a) - 1)) / (a)) * (a))
#define ROUND_DOWN(x, a) (((x) / (a)) * (a))
#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))

/* zephyr/device.h, zephyr/sys/sys_io.h: no registers, every read is 0 */
struct device {
	const char *name;
};
typedef uintptr_t mm_reg_t;
static inline uint32_t sys_read32(mm_reg_t a)
{
	ARG_UNUSED(a);
	return 0;
}
static inline void sys_write32(uint32_t v, mm_reg_t a)
{
	ARG_UNUSED(v);
	ARG_UNUSED(a);
}
static inline bool device_is_ready(const struct device *dev)
{
	return dev != NULL;
}

/* zephyr/cache.h */
static inline void sys_cache_data_flush_range(void *p, size_t n)
{
	ARG_UNUSED(p);
	ARG_UNUSED(n);
}
static inline void sys_cache_data_invd_range(void *p, size_t n)
{
	ARG_UNUSED(p);
	ARG_UNUSED(n);
}

/* zephyr/kernel/mm.h: wait_model.c maps the carve-out from the heap */
#define K_MEM_CACHE_WB 0u
#define K_MEM_PERM_RW 0u
void k_mem_map_phys_bare(uint8_t **virt, uintptr_t phys, size_t size, uint32_t flags);

/* zephyr/logging/log.h: lines on stdout unless quiet, errors counted */
extern unsigned int model_log_errors;
extern bool model_log_quiet;
#define LOG_MODULE_DECLARE(...)
#define MODEL_LOG(lvl, ...)                                                                        \
	do {                                                                                       \
		if (!model_log_quiet) {                                                            \
			printf("      [%s] ", lvl);                                                \
			printf(__VA_ARGS__);                                                       \
			printf("\n");                                                              \
		}                                                                                  \
	} while (0)
#define LOG_ERR(...)                                                                               \
	do {                                                                                       \
		model_log_errors++;                                                                \
		MODEL_LOG("err", __VA_ARGS__);                                                     \
	} while (0)
#define LOG_WRN(...) MODEL_LOG("wrn", __VA_ARGS__)
#define LOG_INF(...) MODEL_LOG("inf", __VA_ARGS__)
#define LOG_DBG(...)                                                                               \
	do {                                                                                       \
	} while (0)

/* The simulated clock, and the step that moves it (wait_model.c): run every
 * scripted interrupt due at or before `until_us` in order, stopping early
 * after one that leaves `sem` (NULL: none) with a count, and return the time
 * reached. */
extern int64_t model_us;
struct k_sem;
int64_t model_advance(int64_t until_us, struct k_sem *sem);

typedef struct {
	int64_t us; /* -1 for ever */
} k_timeout_t;
#define K_FOREVER ((k_timeout_t){ .us = -1 })
#define K_NO_WAIT ((k_timeout_t){ .us = 0 })
#define K_MSEC(ms) ((k_timeout_t){ .us = (int64_t)(ms) * 1000 })

static inline int64_t k_uptime_get(void)
{
	return model_us / 1000;
}
static inline uint32_t k_uptime_get_32(void)
{
	return (uint32_t)k_uptime_get();
}
/* gc_now_ns(): one cycle a microsecond */
static inline uint64_t k_cycle_get_64(void)
{
	return (uint64_t)model_us;
}
static inline uint64_t k_cyc_to_ns_floor64(uint64_t c)
{
	return c * 1000u;
}

/* One thread: a lock is never contended. */
struct k_mutex {
	int held;
};
static inline void k_mutex_init(struct k_mutex *m)
{
	m->held = 0;
}
static inline int k_mutex_lock(struct k_mutex *m, k_timeout_t t)
{
	ARG_UNUSED(t);
	m->held++;
	return 0;
}
static inline int k_mutex_unlock(struct k_mutex *m)
{
	m->held--;
	return 0;
}
struct k_spinlock {
	int held;
};
typedef int k_spinlock_key_t;
static inline k_spinlock_key_t k_spin_lock(struct k_spinlock *l)
{
	l->held++;
	return 0;
}
static inline void k_spin_unlock(struct k_spinlock *l, k_spinlock_key_t k)
{
	ARG_UNUSED(k);
	l->held--;
}
#define K_SPINLOCK(lck)                                                                            \
	for (k_spinlock_key_t __k = k_spin_lock(lck), __once = 1; __once;                          \
	     k_spin_unlock(lck, __k), __once = 0)

struct k_sem {
	unsigned int count, limit;
};
static inline void k_sem_init(struct k_sem *s, unsigned int init, unsigned int limit)
{
	s->count = init;
	s->limit = limit;
}
static inline void k_sem_give(struct k_sem *s)
{
	if (s->count < s->limit) {
		s->count++;
	}
}
/* 0 when taken, -EBUSY for K_NO_WAIT, -EAGAIN at the deadline (as Zephyr) */
static inline int k_sem_take(struct k_sem *s, k_timeout_t t)
{
	if (s->count == 0u && t.us != 0) {
		model_us = model_advance(t.us < 0 ? INT64_MAX : model_us + t.us, s);
	}
	if (s->count == 0u) {
		return t.us == 0 ? -EBUSY : -EAGAIN;
	}
	s->count--;
	return 0;
}
static inline int32_t k_sleep(k_timeout_t t)
{
	model_us = model_advance(model_us + t.us, NULL);
	return 0;
}

#endif /* WAIT_MODEL_ZEPHYR_KERNEL_H */
