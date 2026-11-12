/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * sim_kernel.c: the host model's Zephyr kernel (shim/zephyr/kernel.h) on a
 * simulated clock, so that the model's result never depends on the host.
 *
 * Each model thread is a POSIX thread, but only one runs at a time: the one
 * that holds s_cpu and is s_cur. It runs until it waits (a mutex, a condvar,
 * a semaphore, or model_sleep_until_us). Then the CPU goes to the thread that
 * became ready first. When no thread is ready, the clock jumps to the
 * earliest deadline of a waiting thread, and that thread wakes up timed out.
 * So simulated time passes only while every thread waits, by exactly what
 * the model asks for (the draw, the panel period), and a run gives the same
 * numbers on every host. A woken thread runs when the running one waits (it
 * is not preempted): no simulated time passes in between, so the order only
 * changes which thread takes a lock first, never a period count.
 *
 * When every thread waits for ever, nothing can happen again: the model
 * prints that and exits 3 (on the board that is a hang).
 */
#include <zephyr/kernel.h>

#define MODEL_THREADS 4

enum model_state { T_FREE, T_RUN, T_READY, T_WAIT, T_DONE };

struct model_thread {
	pthread_cond_t turn; /* signalled when the thread gets the CPU */
	enum model_state state;
	const void *on;      /* T_WAIT: the object waited on; NULL for a sleep */
	int64_t deadline;    /* T_WAIT: simulated us, -1 for none */
	uint64_t order;      /* when it began to wait or became ready (FIFO) */
	bool timed_out;
	k_thread_entry_t fn;
	void *p1, *p2, *p3;
};

static pthread_mutex_t s_cpu = PTHREAD_MUTEX_INITIALIZER;
static struct model_thread s_thr[MODEL_THREADS];
static struct model_thread *s_cur;
static int64_t s_now;
static uint64_t s_order;

int64_t model_now_us(void)
{
	return s_now;
}

/* Give the CPU to the next thread; s_cur has left T_RUN. */
static void run_next(void)
{
	struct model_thread *next = NULL;

	for (int i = 0; i < MODEL_THREADS; i++) {
		struct model_thread *t = &s_thr[i];

		if (t->state == T_READY && (next == NULL || t->order < next->order)) {
			next = t;
		}
	}
	if (next == NULL) {
		for (int i = 0; i < MODEL_THREADS; i++) {
			struct model_thread *t = &s_thr[i];

			if (t->state == T_WAIT && t->deadline >= 0 &&
			    (next == NULL || t->deadline < next->deadline ||
			     (t->deadline == next->deadline && t->order < next->order))) {
				next = t;
			}
		}
		if (next == NULL) {
			fprintf(stderr, "latch_model: every thread waits for ever (at %lld us)\n",
				(long long)s_now);
			exit(3);
		}
		if (next->deadline > s_now) {
			s_now = next->deadline;
		}
		next->timed_out = true;
	}
	next->state = T_RUN;
	s_cur = next;
	pthread_cond_signal(&next->turn);
}

static void wait_turn(struct model_thread *me)
{
	while (s_cur != me) {
		pthread_cond_wait(&me->turn, &s_cpu);
	}
}

/* Wait on `on` until woken (0) or until simulated time `deadline` (-EAGAIN;
 * -1 waits for ever). */
static int block(const void *on, int64_t deadline)
{
	struct model_thread *me = s_cur;

	me->state = T_WAIT;
	me->on = on;
	me->deadline = deadline;
	me->timed_out = false;
	me->order = ++s_order;
	run_next();
	wait_turn(me);
	me->on = NULL;
	return me->timed_out ? -EAGAIN : 0;
}

static void make_ready(struct model_thread *t)
{
	t->state = T_READY;
	t->order = ++s_order;
}

/* Wake the threads waiting on `on`: the one that waits longest, or all.
 * Returns how many woke. */
static int wake(const void *on, bool all)
{
	struct model_thread *first = NULL;
	int n = 0;

	for (int i = 0; i < MODEL_THREADS; i++) {
		struct model_thread *t = &s_thr[i];

		if (t->state != T_WAIT || t->on != on) {
			continue;
		}
		if (all) {
			make_ready(t);
			n++;
		} else if (first == NULL || t->order < first->order) {
			first = t;
		}
	}
	if (first != NULL) {
		make_ready(first);
		n = 1;
	}
	return n;
}

void model_kernel_init(void)
{
	for (int i = 0; i < MODEL_THREADS; i++) {
		pthread_cond_init(&s_thr[i].turn, NULL);
	}
	pthread_mutex_lock(&s_cpu);
	s_thr[0].state = T_RUN; /* the main thread */
	s_cur = &s_thr[0];
}

void model_sleep_until_us(int64_t t)
{
	if (t > s_now) {
		(void)block(NULL, t);
	}
}

int k_mutex_lock(struct k_mutex *mu, k_timeout_t t)
{
	ARG_UNUSED(t);
	while (mu->owner != NULL) {
		(void)block(mu, -1);
	}
	mu->owner = s_cur;
	return 0;
}

int k_mutex_unlock(struct k_mutex *mu)
{
	mu->owner = NULL;
	(void)wake(mu, false);
	return 0;
}

int k_condvar_wait(struct k_condvar *cv, struct k_mutex *mu, k_timeout_t t)
{
	int rc;

	if (t.abs_us >= 0 && t.abs_us <= s_now) {
		return -EAGAIN;
	}
	k_mutex_unlock(mu);
	rc = block(cv, t.abs_us);
	k_mutex_lock(mu, K_FOREVER);
	return rc;
}

int k_condvar_broadcast(struct k_condvar *cv)
{
	return wake(cv, true);
}

int k_sem_take(struct k_sem *s, k_timeout_t t)
{
	while (s->count == 0u) {
		if (t.abs_us == 0) {
			return -EBUSY;
		}
		if ((t.abs_us > 0 && t.abs_us <= s_now) || block(s, t.abs_us) != 0) {
			return -EAGAIN;
		}
	}
	s->count--;
	return 0;
}

void k_sem_give(struct k_sem *s)
{
	if (s->count < s->limit) {
		s->count++;
	}
	(void)wake(s, false);
}

static void *thread_main(void *arg)
{
	struct model_thread *me = arg;

	pthread_mutex_lock(&s_cpu);
	wait_turn(me);
	me->fn(me->p1, me->p2, me->p3);
	me->state = T_DONE;
	run_next();
	pthread_mutex_unlock(&s_cpu);
	return NULL;
}

/* The new thread is ready: it runs when the creator waits. */
k_tid_t k_thread_create(struct k_thread *th, char *stack, size_t size, k_thread_entry_t fn,
			void *p1, void *p2, void *p3, int prio, uint32_t options,
			k_timeout_t delay)
{
	struct model_thread *t = NULL;

	ARG_UNUSED(stack);
	ARG_UNUSED(size);
	ARG_UNUSED(prio);
	ARG_UNUSED(options);
	ARG_UNUSED(delay);
	for (int i = 0; i < MODEL_THREADS && t == NULL; i++) {
		if (s_thr[i].state == T_FREE) {
			t = &s_thr[i];
		}
	}
	if (t == NULL) {
		fprintf(stderr, "latch_model: more than %d threads\n", MODEL_THREADS);
		exit(3);
	}
	t->fn = fn;
	t->p1 = p1;
	t->p2 = p2;
	t->p3 = p3;
	make_ready(t);
	if (pthread_create(&th->t, NULL, thread_main, t) != 0) {
		fprintf(stderr, "latch_model: pthread_create failed\n");
		exit(3);
	}
	pthread_detach(th->t);
	return th;
}
