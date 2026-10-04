/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The frame ring: how the A53 rich UI hands frames to the M7 safety core on
 * the Verdin iMX8M Plus.
 *
 * On the AM62P the R5F owns two DSS planes and the A53 hands frames over by
 * physical address through a mailbox doorbell plus a shared-memory ring
 * (include/safeui/ipc.h, struct safeui_frame). The i.MX8M Plus has one LCDIF
 * plane and no blender, so the M7 composites in software and flips the
 * scan-out buffer itself (platform/fw_imx8mp/docs/integration.md). The message
 * is unchanged -- this file embeds `struct safeui_frame` verbatim -- but the
 * transport is not: both cores run Zephyr, both see the same non-cacheable
 * page of DDR, and the M7 polls every tick anyway, so there is no doorbell,
 * no mailbox and no interrupt.
 *
 * One producer (the A53), one consumer (the M7), no locks. The whole protocol
 * is the `produced` counter: a slot is complete when, and only when, the
 * counter that names it has been incremented.
 *
 * Both sides include this header, so neither can drift from the other. The
 * A53 side may live in another repository; it needs this header and nothing
 * else from here.
 *
 * Cacheability: the ring itself sits in memory both cores map non-cacheable
 * (on the M7, the window platform/fw_imx8mp/src/m7_mpu.c carves out), so the
 * counters need no maintenance. The frame buffers the ring points at are
 * cacheable on both sides, and the rule is: the producer flushes what it
 * wrote before it publishes, the consumer invalidates before it reads.
 * Neither may rely on the other's cache state.
 */
#ifndef SAFEUI_FRAME_RING_H_
#define SAFEUI_FRAME_RING_H_

#include <stdbool.h>
#include <stdint.h>
#include <safeui/ipc.h>

#ifdef __cplusplus
extern "C" {
#endif

/* 'FRM7' little-endian: the A53 writes it last, once the ring is set up. */
#define SAFEUI_FRAME_RING_MAGIC 0x374d5246u
/*
 * 2: consumer_writes appended (2026-09-21).
 * 3: the M7's page (struct safeui_m7_page below): the vehicle state and the
 *    background now come from the M7 (2026-10-03). The ring's own layout did
 *    not change; the version did, because a version-2 producer renders its
 *    own vehicle state and expects no background, and showing its frames
 *    under an M7 that publishes different values would put two speeds on one
 *    panel. The M7 refuses such a producer with its own verdict
 *    (APP_FRAME_OLD_PRODUCER) rather than calling it "no ring".
 */
#define SAFEUI_FRAME_RING_VERSION 3u

/*
 * Three slots, for the same reason the AM62P compositor uses three buffers:
 * the producer must always have one that neither the consumer nor the display
 * is reading.
 */
#define SAFEUI_FRAME_RING_SLOTS 3u

/*
 * How long the M7 waits before it decides the A53 is gone. The same value
 * lib/platform_core uses for the mailbox heartbeat on the AM62P, so the two
 * ports behave alike.
 */
#define SAFEUI_FRAME_RING_TIMEOUT_MS 1500u

struct safeui_frame_ring {
	uint32_t magic;    /* SAFEUI_FRAME_RING_MAGIC once the A53 has attached */
	uint32_t version;  /* SAFEUI_FRAME_RING_VERSION */
	uint32_t slots;    /* SAFEUI_FRAME_RING_SLOTS, so a reader can check */
	uint32_t produced; /* +1 once a slot is fully written; the newest
			    * complete frame is (produced - 1) % slots */
	struct safeui_frame slot[SAFEUI_FRAME_RING_SLOTS];
	uint32_t a53_alive; /* +1 every A53 tick: this is the heartbeat */
	uint32_t consumed;  /* the M7 writes: seq of the last frame composited */
	uint32_t shown;     /* the M7 writes: seq of the frame now scanning out */
	/*
	 * The M7 writes this; the producer only reads it. It is what makes it
	 * safe for the consumer to composite *into* a published buffer instead
	 * of copying it somewhere else first -- which is how the M7 meets the
	 * panel's 60 Hz, because a 1280x800 copy it does not do is 4 MiB of
	 * DDR traffic it does not pay for.
	 *
	 * Note what is NOT here: a field saying which buffers the consumer is
	 * using. `shown` already says it. A buffer holding sequence S is the
	 * producer's again once `shown` has moved past S, and that covers both
	 * the buffer being composited (nothing is shown past it yet) and the
	 * one the LCDIF is still scanning (`shown` names it, and "past" is
	 * strict). Two rules for one thing is how two cores come to disagree.
	 */
	uint32_t consumer_writes; /* 1 = the M7 blends the safety layer into the
				   * buffer the producer published, so that
				   * buffer's contents are undefined from the
				   * publish onwards. A producer that keeps
				   * pixels between frames (a partial redraw)
				   * must not do so for this ring. */
};

/*
 * The compiler must not move the slot's stores after the `produced` store, and
 * on the A53 neither may the processor. A release barrier costs nothing here
 * and is the whole of the synchronisation.
 */
#if defined(__aarch64__) || defined(__arm__)
#define SAFEUI_FRAME_RING_BARRIER() __asm__ volatile("dmb ish" ::: "memory")
#else
#define SAFEUI_FRAME_RING_BARRIER() __asm__ volatile("" ::: "memory")
#endif

/* Is there a ring here, and one this build understands? Both halves ask. */
static inline bool safeui_frame_ring_attached(const volatile struct safeui_frame_ring *r)
{
	return r->magic == SAFEUI_FRAME_RING_MAGIC &&
	       r->version == SAFEUI_FRAME_RING_VERSION && r->slots == SAFEUI_FRAME_RING_SLOTS;
}

/* ---------------------------------------------------------- producer (A53) */

/*
 * Set the ring up. Call once, before the first publish. Clears the counters
 * and writes the magic last, so a consumer that sees the magic sees a ring
 * that is ready. `consumed` and `shown` belong to the M7 and are cleared here
 * only because nobody has written them yet.
 */
static inline void safeui_frame_ring_init(volatile struct safeui_frame_ring *r)
{
	r->version = SAFEUI_FRAME_RING_VERSION;
	r->slots = SAFEUI_FRAME_RING_SLOTS;
	r->produced = 0u;
	r->a53_alive = 0u;
	r->consumed = 0u;
	r->shown = 0u;
	/*
	 * `consumer_writes` belongs to the M7 and is deliberately not cleared
	 * here: it is a property of the consumer, which a producer restart
	 * does not change.
	 */
	for (unsigned int i = 0; i < SAFEUI_FRAME_RING_SLOTS; i++) {
		r->slot[i].seq = 0u;
		r->slot[i].pa = 0u;
		r->slot[i].app_alive = 0u;
	}
	SAFEUI_FRAME_RING_BARRIER();
	r->magic = SAFEUI_FRAME_RING_MAGIC;
}

/*
 * Which buffer to render the next frame into: the slot the next publish will
 * claim. With three slots and one in-flight consumer this is never a buffer
 * anyone is reading.
 */
static inline unsigned int safeui_frame_ring_next_slot(const volatile struct safeui_frame_ring *r)
{
	return (unsigned int)(r->produced % SAFEUI_FRAME_RING_SLOTS);
}

/*
 * Does the consumer write into the buffers this ring publishes?
 *
 * When it does, a published buffer comes back with the safety layer blended
 * into it, so a producer that would otherwise redraw only what changed must
 * redraw in full. Answered by the consumer at its own boot; a ring whose
 * consumer never said is assumed not to, which is the old behaviour.
 */
static inline bool safeui_frame_ring_consumer_writes(const volatile struct safeui_frame_ring *r)
{
	return safeui_frame_ring_attached(r) && r->consumer_writes != 0u;
}

/*
 * Publish a finished frame and return its sequence number. `f` describes the
 * buffer just rendered; its `seq` is filled in here, so the two sides cannot
 * disagree about numbering.
 *
 * The caller must have finished writing the buffer, including flushing it --
 * only the caller knows how much of it changed.
 */
static inline uint32_t safeui_frame_ring_publish(volatile struct safeui_frame_ring *r,
						 const struct safeui_frame *f)
{
	const unsigned int at = safeui_frame_ring_next_slot(r);
	const uint32_t seq = r->produced + 1u;

	r->slot[at].pa = f->pa;
	r->slot[at].width = f->width;
	r->slot[at].height = f->height;
	r->slot[at].stride_bytes = f->stride_bytes;
	r->slot[at].format = f->format;
	r->slot[at].app_alive = f->app_alive;
	r->slot[at].seq = seq;

	/* Everything above must land before the counter that publishes it. */
	SAFEUI_FRAME_RING_BARRIER();
	r->produced = seq;
	return seq;
}

/*
 * The heartbeat. Call at least ten times per SAFEUI_FRAME_RING_TIMEOUT_MS from
 * the A53's own periodic work -- not from the render loop, because the point
 * is to say the core is alive even when it has no frame to show.
 */
static inline void safeui_frame_ring_heartbeat(volatile struct safeui_frame_ring *r)
{
	r->a53_alive++;
}

/* What the M7 has put on the panel, for the A53's own triple buffering. */
static inline uint32_t safeui_frame_ring_shown(const volatile struct safeui_frame_ring *r)
{
	return r->shown;
}

/* ------------------------------------------------------------ consumer (M7) */

/*
 * The newest complete frame, or false when there is none. Older frames are
 * skipped rather than queued: a safety display shows the most recent picture,
 * never a backlog. The caller invalidates the buffer before reading it.
 */
static inline bool safeui_frame_ring_take(const volatile struct safeui_frame_ring *r,
					  struct safeui_frame *out)
{
	uint32_t produced;
	unsigned int at;

	if (!safeui_frame_ring_attached(r)) {
		return false;
	}
	produced = r->produced;
	if (produced == 0u) {
		return false;
	}
	/* Read the counter before the slot it names. */
	SAFEUI_FRAME_RING_BARRIER();
	at = (unsigned int)((produced - 1u) % SAFEUI_FRAME_RING_SLOTS);
	out->pa = r->slot[at].pa;
	out->width = r->slot[at].width;
	out->height = r->slot[at].height;
	out->stride_bytes = r->slot[at].stride_bytes;
	out->format = r->slot[at].format;
	out->app_alive = r->slot[at].app_alive;
	out->seq = r->slot[at].seq;

	/*
	 * If the producer came all the way round while we were copying, the
	 * slot we read is no longer the one the counter named. Three slots and
	 * a consumer that runs every tick make that impossible in practice;
	 * checking costs one compare and turns "impossible" into "detected".
	 */
	return out->seq == produced;
}

/*
 * Whether the A53 is still there. The two scratch values belong to the caller,
 * which is what lets this stay a header: the M7 keeps them in its own state
 * and calls this once a tick. Before the first beat ever arrives the answer is
 * false, which is the right answer -- there is no application yet.
 */
static inline bool safeui_frame_ring_alive(const volatile struct safeui_frame_ring *r,
					   uint32_t now_ms, uint32_t *last_beat,
					   uint32_t *last_change_ms)
{
	const uint32_t beat = safeui_frame_ring_attached(r) ? r->a53_alive : 0u;

	if (beat != *last_beat) {
		*last_beat = beat;
		*last_change_ms = now_ms;
	}
	return beat != 0u && (now_ms - *last_change_ms) < SAFEUI_FRAME_RING_TIMEOUT_MS;
}

/* The M7 reports back: composited, then actually on the panel. */
static inline void safeui_frame_ring_consumed(volatile struct safeui_frame_ring *r, uint32_t seq)
{
	r->consumed = seq;
}

/*
 * Say once, at the consumer's boot, that it composites into the buffers it is
 * given. See safeui_frame_ring_consumer_writes().
 */
static inline void safeui_frame_ring_declare_writes(volatile struct safeui_frame_ring *r, bool on)
{
	r->consumer_writes = on ? 1u : 0u;
}

static inline void safeui_frame_ring_set_shown(volatile struct safeui_frame_ring *r, uint32_t seq)
{
	SAFEUI_FRAME_RING_BARRIER();
	r->shown = seq;
}

/*
 * ============================================ the M7's page (ring version 3)
 *
 * What travels the other way: the M7 publishes, the A53 only reads.
 *
 * Two things, both decided with the user on 2026-10-03:
 *
 *   - The vehicle state. The safety core computes it (lib/platform_core's
 *     model today; a bus later) and draws the speed, gear and telltales from
 *     it itself. The rich UI reads the SAME values here instead of running a
 *     model of its own, so the A53's needle and the M7's digits cannot
 *     disagree about how fast the car is going.
 *
 *   - The background. The M7 renders the cluster's background picture ONCE,
 *     at boot, into a buffer of its own and describes it here. The A53 draws
 *     that picture as the first thing in every frame and its UI over it; the
 *     M7 then blends its safety layer over the result, as before. On the
 *     panel: M7 background, A53 UI, M7 safety elements. A full-screen
 *     background redrawn per frame costs the A53's GPU ~22 ms and the M7's
 *     compositor 38 ms, so neither can afford to draw it every frame; the A53
 *     copying a finished picture is what is affordable.
 *
 * Where: in the ring's own 4 KiB page, SAFEUI_M7_PAGE_OFFSET bytes past the
 * ring, so both cores already map it non-cacheable and nothing new has to be
 * carved out. The ring is ~130 bytes; the A53 never writes past it, and the M7
 * never writes the ring's producer fields, so the two halves of the page have
 * one writer each.
 *
 * Every field is a 32-bit word, so the layout needs no packing rules and is
 * the same for the 32-bit M7 and the 64-bit A53 (the static asserts below pin
 * it). Append only, like the M7's status block.
 */
#define SAFEUI_M7_PAGE_OFFSET 0x800u
#define SAFEUI_M7_PAGE_MAGIC 0x4750374du /* 'M7PG' little-endian, written last */
#define SAFEUI_M7_PAGE_VERSION 1u

/* struct safeui_vehicle_state.valid: one bit per value the model vouches for.
 * A value whose bit is clear must be drawn as "unknown", never as a number. */
#define SAFEUI_VEHICLE_VALID_SPEED (1u << 0)
#define SAFEUI_VEHICLE_VALID_RPM (1u << 1)
#define SAFEUI_VEHICLE_VALID_GEAR (1u << 2)
#define SAFEUI_VEHICLE_VALID_FUEL (1u << 3)
#define SAFEUI_VEHICLE_VALID_TANK (1u << 4)
#define SAFEUI_VEHICLE_VALID_ODOMETER (1u << 5)
#define SAFEUI_VEHICLE_VALID_TRIP (1u << 6)
#define SAFEUI_VEHICLE_VALID_INDICATOR (1u << 7)

/*
 * struct safeui_vehicle_state.lamps. The indicator is two facts, kept apart on
 * purpose: which way the driver has signalled (the switch), and whether the
 * flasher is lit at this instant (the phase). A lamp is drawn lit when both
 * are set. Sending the product would make the A53 run a flasher of its own,
 * which then drifts against the M7's -- the very disagreement this page
 * exists to remove.
 */
#define SAFEUI_VEHICLE_LAMP_TURN_LEFT (1u << 0)  /* indicator switch: left */
#define SAFEUI_VEHICLE_LAMP_TURN_RIGHT (1u << 1) /* indicator switch: right */
#define SAFEUI_VEHICLE_LAMP_BLINK_ON (1u << 2)   /* flasher phase: lit now */
#define SAFEUI_VEHICLE_LAMP_LOW_BEAM (1u << 3)
#define SAFEUI_VEHICLE_ENGINE_RUNNING (1u << 4)  /* the READY pill */
#define SAFEUI_VEHICLE_IGNITION (1u << 5)

/* Where the values came from. Nothing on the A53 changes with it; it is there
 * so that a picture can be told apart from a demo when it matters. */
#define SAFEUI_VEHICLE_SOURCE_NONE 0u   /* never published */
#define SAFEUI_VEHICLE_SOURCE_DEMO 1u   /* the built-in drive cycle */
#define SAFEUI_VEHICLE_SOURCE_INPUTS 2u /* the model, fed by real inputs */

struct safeui_vehicle_state {
	/*
	 * A sequence lock. Odd while the M7 is writing, even when the record
	 * is whole; +2 per publish, so 0 means "never published". The A53
	 * reads gen, the record, then gen again, and keeps the copy only if
	 * both reads are the same even number (safeui_vehicle_read()). The M7
	 * is a different core that does not wait for anybody, so this is the
	 * only way a reader can know it did not get half of one update and
	 * half of the next.
	 */
	uint32_t gen;
	uint32_t time_ms;       /* the M7's uptime when the model produced it */
	int32_t speed_kmh_x10;
	int32_t engine_rpm;
	int32_t gear;           /* -1 R, 0 N, 1.. D<n> */
	int32_t fuel_ml;
	int32_t fuel_tank_ml;
	int32_t odometer_m;
	int32_t trip_m;         /* since the last refuel */
	uint32_t valid;         /* SAFEUI_VEHICLE_VALID_* */
	uint32_t lamps;         /* SAFEUI_VEHICLE_LAMP_*, SAFEUI_VEHICLE_ENGINE_*, ... */
	uint32_t warnings;      /* SAFEUI_WARN_* (safeui/abi.h), as the M7 evaluated them */
	uint32_t source;        /* SAFEUI_VEHICLE_SOURCE_* */
	uint32_t reserved[3];
};

/* struct safeui_background.state */
#define SAFEUI_BG_NONE 0u      /* this M7 renders no background */
#define SAFEUI_BG_RENDERING 1u /* not yet: try again in a moment */
#define SAFEUI_BG_READY 2u     /* every field below is valid and the pixels are in DDR */
#define SAFEUI_BG_FAILED 3u    /* it tried and could not; the A53 clears to black */

struct safeui_background {
	uint32_t state;        /* SAFEUI_BG_*, written last */
	uint32_t gen;          /* +1 each time the picture is rendered: 1 after boot */
	uint32_t pa_lo;        /* where the picture is: physical, in the M7's carveout */
	uint32_t pa_hi;
	uint32_t width;
	uint32_t height;
	uint32_t stride_bytes;
	uint32_t format;       /* struct safeui_frame's codes: 0x7 = XRGB8888, which is
				* B, G, R, X in memory, X = 0xff */
	uint32_t bytes;        /* height * stride_bytes */
	uint32_t render_us;    /* what rendering it cost the M7, once */
	uint32_t checksum;     /* the sum of every 32-bit word of the picture,
				* modulo 2^32, so a reader can tell the pixels it
				* sees are the ones described -- a stale buffer
				* from another boot carries the right descriptor
				* only by accident */
	uint32_t reserved[5];
};

struct safeui_m7_page {
	uint32_t magic;        /* SAFEUI_M7_PAGE_MAGIC once the M7 has set it up */
	uint32_t version;      /* SAFEUI_M7_PAGE_VERSION */
	uint32_t size;         /* sizeof(struct safeui_m7_page), so a reader can check */
	uint32_t ring_version; /* the SAFEUI_FRAME_RING_VERSION this M7 accepts */
	uint32_t reserved[4];
	struct safeui_background bg;
	struct safeui_vehicle_state vehicle;
};

#ifndef __cplusplus
_Static_assert(sizeof(struct safeui_vehicle_state) == 64, "the vehicle state is 16 words");
_Static_assert(sizeof(struct safeui_background) == 64, "the background descriptor is 16 words");
_Static_assert(sizeof(struct safeui_m7_page) == 160, "the M7's page is 40 words");
_Static_assert(SAFEUI_M7_PAGE_OFFSET >= sizeof(struct safeui_frame_ring),
	       "the M7's page must not overlap the ring");
_Static_assert(SAFEUI_M7_PAGE_OFFSET + sizeof(struct safeui_m7_page) <= 0x1000u,
	       "the M7's page must stay inside the ring's 4 KiB page");
#endif

/* The M7's page, given the ring: both live in the same mapped page. */
static inline volatile struct safeui_m7_page *safeui_m7_page_of(volatile struct safeui_frame_ring *r)
{
	return (volatile struct safeui_m7_page *)((volatile uint8_t *)r + SAFEUI_M7_PAGE_OFFSET);
}

/* Is there a page here, and one this build understands? The A53 asks. */
static inline bool safeui_m7_page_ok(const volatile struct safeui_m7_page *p)
{
	return p->magic == SAFEUI_M7_PAGE_MAGIC && p->version == SAFEUI_M7_PAGE_VERSION &&
	       p->size == (uint32_t)sizeof(struct safeui_m7_page) &&
	       p->ring_version == SAFEUI_FRAME_RING_VERSION;
}

/* ------------------------------------------------------- M7 page: writer */

/*
 * Set the page up. The M7 calls this once at boot, before anything else
 * touches it, and owns everything in it from then on. The magic goes first to
 * zero and last to its value, so an A53 that looks in mid-call sees either no
 * page or a complete, empty one (no vehicle state yet, no background yet).
 */
static inline void safeui_m7_page_init(volatile struct safeui_m7_page *p)
{
	volatile uint32_t *w = (volatile uint32_t *)p;

	p->magic = 0u;
	SAFEUI_FRAME_RING_BARRIER();
	for (unsigned int i = 1; i < sizeof(*p) / sizeof(uint32_t); i++) {
		w[i] = 0u;
	}
	p->version = SAFEUI_M7_PAGE_VERSION;
	p->size = (uint32_t)sizeof(*p);
	p->ring_version = SAFEUI_FRAME_RING_VERSION;
	SAFEUI_FRAME_RING_BARRIER();
	p->magic = SAFEUI_M7_PAGE_MAGIC;
}

/* Publish one vehicle state. `v->gen` is ignored: the page owns the count. */
static inline void safeui_vehicle_publish(volatile struct safeui_m7_page *p,
					  const struct safeui_vehicle_state *v)
{
	volatile struct safeui_vehicle_state *d = &p->vehicle;
	const uint32_t gen = d->gen & ~1u;

	d->gen = gen + 1u; /* odd: being written */
	SAFEUI_FRAME_RING_BARRIER();
	d->time_ms = v->time_ms;
	d->speed_kmh_x10 = v->speed_kmh_x10;
	d->engine_rpm = v->engine_rpm;
	d->gear = v->gear;
	d->fuel_ml = v->fuel_ml;
	d->fuel_tank_ml = v->fuel_tank_ml;
	d->odometer_m = v->odometer_m;
	d->trip_m = v->trip_m;
	d->valid = v->valid;
	d->lamps = v->lamps;
	d->warnings = v->warnings;
	d->source = v->source;
	SAFEUI_FRAME_RING_BARRIER();
	d->gen = gen + 2u; /* even again: whole */
}

/*
 * Describe the background. Everything but `state` first, then the state, so a
 * reader that sees SAFEUI_BG_READY sees the rest of it too. The caller has
 * already cleaned the picture out of its own caches.
 */
static inline void safeui_background_publish(volatile struct safeui_m7_page *p,
					     const struct safeui_background *b)
{
	volatile struct safeui_background *d = &p->bg;

	d->state = SAFEUI_BG_RENDERING;
	SAFEUI_FRAME_RING_BARRIER();
	d->gen = b->gen;
	d->pa_lo = b->pa_lo;
	d->pa_hi = b->pa_hi;
	d->width = b->width;
	d->height = b->height;
	d->stride_bytes = b->stride_bytes;
	d->format = b->format;
	d->bytes = b->bytes;
	d->render_us = b->render_us;
	d->checksum = b->checksum;
	SAFEUI_FRAME_RING_BARRIER();
	d->state = b->state;
}

/* ------------------------------------------------------- M7 page: reader */

/*
 * Copy the newest whole vehicle state, or return false when there has never
 * been one or every attempt raced a write. A writer that publishes once per
 * model tick finishes in well under a microsecond, so a few attempts are
 * plenty; a caller that gets false keeps what it had.
 */
static inline bool safeui_vehicle_read(const volatile struct safeui_m7_page *p,
				       struct safeui_vehicle_state *out)
{
	const volatile struct safeui_vehicle_state *s = &p->vehicle;

	for (unsigned int attempt = 0; attempt < 8u; attempt++) {
		const uint32_t g1 = s->gen;

		if (g1 == 0u) {
			return false; /* never published */
		}
		if ((g1 & 1u) != 0u) {
			continue; /* mid-write */
		}
		SAFEUI_FRAME_RING_BARRIER();
		out->time_ms = s->time_ms;
		out->speed_kmh_x10 = s->speed_kmh_x10;
		out->engine_rpm = s->engine_rpm;
		out->gear = s->gear;
		out->fuel_ml = s->fuel_ml;
		out->fuel_tank_ml = s->fuel_tank_ml;
		out->odometer_m = s->odometer_m;
		out->trip_m = s->trip_m;
		out->valid = s->valid;
		out->lamps = s->lamps;
		out->warnings = s->warnings;
		out->source = s->source;
		SAFEUI_FRAME_RING_BARRIER();
		if (s->gen == g1) {
			out->gen = g1;
			for (unsigned int i = 0; i < 3u; i++) {
				out->reserved[i] = 0u;
			}
			return true;
		}
	}
	return false;
}

/* The background's description, once it is READY; false before that. */
static inline bool safeui_background_read(const volatile struct safeui_m7_page *p,
					  struct safeui_background *out)
{
	const volatile struct safeui_background *s = &p->bg;

	out->state = s->state;
	if (out->state != SAFEUI_BG_READY) {
		return false;
	}
	SAFEUI_FRAME_RING_BARRIER();
	out->gen = s->gen;
	out->pa_lo = s->pa_lo;
	out->pa_hi = s->pa_hi;
	out->width = s->width;
	out->height = s->height;
	out->stride_bytes = s->stride_bytes;
	out->format = s->format;
	out->bytes = s->bytes;
	out->render_us = s->render_us;
	out->checksum = s->checksum;
	return true;
}

#ifdef __cplusplus
}
#endif

#endif /* SAFEUI_FRAME_RING_H_ */
