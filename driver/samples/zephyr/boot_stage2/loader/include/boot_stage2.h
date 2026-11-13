/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The C view of yakogl-boot-stage2 (loader/src/lib.rs): the header check,
 * the unpack (CRC-32, then inflate, LZ4 or a copy) and the jump of the
 * second-stage loader. Header versions 1 and 2 describe one image
 * (bs2_unpack); version 3 a table of up to BS2_MAX_SEGMENTS segments, each
 * at its own load address (bs2_segment_unpack). The header layouts are in
 * the crate's header comment; the struct layouts are pinned below and in the
 * crate.
 */
#ifndef BOOT_STAGE2_H
#define BOOT_STAGE2_H

#include <stddef.h>
#include <stdint.h>

#define BS2_HEADER_BYTES 512
#define BS2_COMPRESSION_NONE    0
#define BS2_COMPRESSION_DEFLATE 1
#define BS2_COMPRESSION_LZ4     2
#define BS2_VERSION_SEGMENTS    3 /* bs2_info.version of a segment-table header */
#define BS2_MAX_SEGMENTS        8

/* The progress of bs2_unpack and bs2_segment_unpack, reported through
 * bs2_mark (loader/src/lib.rs). Each segment marks: the stamps that stay are
 * the last segment's. */
#define BS2_MARK_STORED_CHECKED 5
#define BS2_MARK_DECODED        6

struct bs2_limits {
	uint64_t load;          /* the only load address this loader serves (version 3:
				 * segment 0's), the region's start */
	uint64_t max_stored;    /* bytes the source holds after the header */
	uint64_t max_image;     /* versions 1 and 2: bytes the image may have once
				 * decompressed, and the room its zero range ends in */
	uint64_t region_end;    /* version 3: every segment ends at or below this */
	uint64_t staging;       /* version 3: where a compressed eMMC segment is read */
	uint64_t staging_bytes; /* no segment overlaps staging..+this; 0: nothing staged */
};

/* One segment of a version-3 header. */
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

/* Versions 1 and 2 fill the fields up to zero_bytes and leave count 0.
 * Version 3 fills load (segment 0's), entry, stored_bytes (the whole
 * payload), name, count and segments; the other single-image fields are 0. */
struct bs2_info {
	uint64_t load;
	uint64_t entry;
	uint64_t stored_bytes;
	uint64_t image_bytes;
	uint32_t compression;
	uint32_t stored_crc32;
	uint32_t image_crc32;
	char name[33];       /* NUL terminated */
	uint64_t zero_offset; /* a range to zero after the unpack, from load */
	uint64_t zero_bytes;  /* 0: none (header version 1 has none) */
	uint32_t version;     /* 1, 2 or BS2_VERSION_SEGMENTS */
	uint32_t count;       /* segments[] in use (version 3); 0 for versions 1 and 2 */
	struct bs2_segment segments[BS2_MAX_SEGMENTS];
};

_Static_assert(offsetof(struct bs2_limits, max_image) == 16, "bs2_limits layout");
_Static_assert(offsetof(struct bs2_limits, region_end) == 24, "bs2_limits layout");
_Static_assert(sizeof(struct bs2_limits) == 48, "bs2_limits layout");
_Static_assert(sizeof(struct bs2_segment) == 56, "bs2_segment layout");
_Static_assert(offsetof(struct bs2_segment, compression) == 40, "bs2_segment layout");
_Static_assert(offsetof(struct bs2_segment, zero_bytes) == 52, "bs2_segment layout");
_Static_assert(offsetof(struct bs2_info, compression) == 32, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, name) == 44, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, zero_offset) == 80, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, version) == 96, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, count) == 100, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, segments) == 104, "bs2_info layout");
_Static_assert(sizeof(struct bs2_info) == 552, "bs2_info layout");

/* 0, or a negative code bs2_strerror() names. A version-3 header may cut the
 * image into parts (segment 0 and the segments after it that lie inside its
 * footprint, loader/src/lib.rs); the check guarantees that no segment's zero
 * bytes share a byte with another segment's image bytes or zero bytes, or
 * with the staging buffer, so they can be cleared while the segments are
 * read. */
int bs2_header_parse(const uint8_t *sector, size_t len, const struct bs2_limits *limits,
		     struct bs2_info *out);
/* Versions 1 and 2: the image from its stored bytes. Decoded into image, or,
 * uncompressed, checked where it is when image == stored (read in place)
 * and copied to image otherwise. */
int bs2_unpack(const struct bs2_info *info, const uint8_t *stored, uint8_t *image, size_t image_cap);
/* Version 3: segment `index` from its stored bytes at `stored` to its load
 * address (decode or copy), or, when stored == the load address (an
 * uncompressed segment read in place), only its check. */
int bs2_segment_unpack(const struct bs2_info *info, uint32_t index, const uint8_t *stored);
uint32_t bs2_crc32(const uint8_t *data, size_t len);
const char *bs2_strerror(int code);

/* Interrupts masked, devices quiet, whole D-cache cleaned and invalidated:
 * MMU and caches off, branch to entry at EL1. Never returns. */
__attribute__((noreturn)) void bs2_jump(uint64_t entry);

/* Called by bs2_unpack and bs2_segment_unpack at BS2_MARK_*; the C side
 * stamps the time. */
void bs2_mark(uint32_t stage);

/* Called by the crate's panic handler; the C side prints and stops. */
__attribute__((noreturn)) void bs2_panic(const char *file, size_t file_len, uint32_t line);

#endif /* BOOT_STAGE2_H */
