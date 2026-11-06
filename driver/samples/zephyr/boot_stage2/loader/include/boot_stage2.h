/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The C view of yakogl-boot-stage2 (loader/src/lib.rs): the header check,
 * the unpack (CRC-32 + inflate) and the jump of the AM62P second-stage
 * loader. The layouts are pinned below and in the crate.
 */
#ifndef BOOT_STAGE2_H
#define BOOT_STAGE2_H

#include <stddef.h>
#include <stdint.h>

#define BS2_HEADER_BYTES 512
#define BS2_COMPRESSION_NONE    0
#define BS2_COMPRESSION_DEFLATE 1
#define BS2_COMPRESSION_LZ4     2

/* bs2_unpack's progress, reported through bs2_mark (loader/src/lib.rs). */
#define BS2_MARK_STORED_CHECKED 5
#define BS2_MARK_DECODED        6

struct bs2_limits {
	uint64_t load;       /* the only load address this loader serves */
	uint64_t max_stored; /* bytes the user-area range holds after the header */
	uint64_t max_image;  /* bytes the image may have once decompressed */
};

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
};

_Static_assert(sizeof(struct bs2_limits) == 24, "bs2_limits layout");
_Static_assert(offsetof(struct bs2_info, compression) == 32, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, name) == 44, "bs2_info layout");
_Static_assert(offsetof(struct bs2_info, zero_offset) == 80, "bs2_info layout");
_Static_assert(sizeof(struct bs2_info) == 96, "bs2_info layout");

/* 0, or a negative code bs2_strerror() names. */
int bs2_header_parse(const uint8_t *sector, size_t len, const struct bs2_limits *limits,
		     struct bs2_info *out);
int bs2_unpack(const struct bs2_info *info, const uint8_t *stored, uint8_t *image, size_t image_cap);
uint32_t bs2_crc32(const uint8_t *data, size_t len);
const char *bs2_strerror(int code);

/* Interrupts masked, devices quiet, whole D-cache cleaned and invalidated:
 * MMU and caches off, branch to entry at EL1. Never returns. */
__attribute__((noreturn)) void bs2_jump(uint64_t entry);

/* Called by bs2_unpack at BS2_MARK_*; the C side stamps the time. */
void bs2_mark(uint32_t stage);

/* Called by the crate's panic handler; the C side prints and stops. */
__attribute__((noreturn)) void bs2_panic(const char *file, size_t file_len, uint32_t line);

#endif /* BOOT_STAGE2_H */
