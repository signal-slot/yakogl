/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The C view of yakogl-boot-stage2 (loader/src/lib.rs): the header check,
 * the segment unpack (CRC-32 + inflate or copy) and the jump of the
 * second-stage loader. The layouts are pinned below and in the crate.
 */
#ifndef BOOT_STAGE2_H
#define BOOT_STAGE2_H

#include <stddef.h>
#include <stdint.h>

#define BS2_HEADER_BYTES 512
#define BS2_COMPRESSION_NONE    0
#define BS2_COMPRESSION_DEFLATE 1
#define BS2_MAX_SEGMENTS 8

struct bs2_limits {
	uint64_t load;          /* segment 0's (the image's) load address, the region's start */
	uint64_t region_end;    /* every segment ends at or below this (exclusive) */
	uint64_t max_stored;    /* bytes the source holds after the header */
	uint64_t staging;       /* where a compressed eMMC payload is read */
	uint64_t staging_bytes; /* no segment overlaps staging..+this; 0: nothing staged */
};

struct bs2_segment {
	uint64_t load;
	uint64_t image_bytes;
	uint64_t footprint;     /* RAM the segment owns from load (>= image_bytes) */
	uint64_t stored_offset; /* from the first byte after the header, sector aligned */
	uint64_t stored_bytes;
	uint32_t compression;
	uint32_t stored_crc32;
	uint32_t image_crc32;
	uint32_t zero_bytes;    /* RAM after the image bytes (load + image_bytes ..) that the
				 * loader clears; inside the footprint. The caller of
				 * bs2_segment_unpack clears it, not the crate. */
};

struct bs2_info {
	struct bs2_segment segments[BS2_MAX_SEGMENTS];
	uint64_t entry;
	uint64_t stored_bytes;  /* the payload after the header, all segments */
	uint32_t version;       /* 1: one image; 2: the segment table */
	uint32_t count;
	char name[33];          /* NUL terminated */
};

_Static_assert(sizeof(struct bs2_limits) == 40, "bs2_limits layout");
_Static_assert(sizeof(struct bs2_segment) == 56, "bs2_segment layout");
_Static_assert(offsetof(struct bs2_segment, compression) == 40, "bs2_segment layout");
_Static_assert(offsetof(struct bs2_segment, zero_bytes) == 52, "bs2_segment layout");
_Static_assert(offsetof(struct bs2_info, entry) == 448, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, version) == 464, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, name) == 472, "bs2_info layout");
_Static_assert(sizeof(struct bs2_info) == 512, "bs2_info layout");

/* 0, or a negative code bs2_strerror() names. A version-2 header may cut the
 * image into parts (segment 0 and the segments after it that lie inside its
 * footprint, loader/src/lib.rs); the check guarantees that no segment's zero
 * range overlaps another segment's image bytes or zero range, or the staging
 * buffer, so the ranges can be cleared while the segments are read. */
int bs2_header_parse(const uint8_t *sector, size_t len, const struct bs2_limits *limits,
		     struct bs2_info *out);
/* Segment `index` from its stored bytes at `stored` to its load address
 * (inflate or copy), or, when stored == the load address (an uncompressed
 * segment read in place), only its check. */
int bs2_segment_unpack(const struct bs2_info *info, uint32_t index, const uint8_t *stored);
uint32_t bs2_crc32(const uint8_t *data, size_t len);
const char *bs2_strerror(int code);

/* Interrupts masked, devices quiet, whole D-cache cleaned and invalidated:
 * MMU and caches off, branch to entry at EL1. Never returns. */
__attribute__((noreturn)) void bs2_jump(uint64_t entry);

/* Called by the crate's panic handler; the C side prints and stops. */
__attribute__((noreturn)) void bs2_panic(const char *file, size_t file_len, uint32_t line);

#endif /* BOOT_STAGE2_H */
