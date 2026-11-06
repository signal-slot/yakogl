/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_dtb: prints the device tree the Raspberry Pi 5 firmware handed to the
 * image in x0 (saved by boot_x0.S) and the nodes of it that matter to the GPU
 * port: path, compatible and reg of every node whose compatible names V3D, the
 * PM block or the VC4 display side.
 *
 * The DTB lies above the image RAM, which is all Zephyr's rpi_5 MMU map
 * covers, so it is mapped here with device_map() (normal cacheable memory:
 * the walker and the string functions may make unaligned accesses, which
 * device memory forbids) before anything reads it. First the page holding the
 * header, to learn totalsize, then the whole blob, which fdt_walk() checks
 * before this file trusts a token of it. The watchdog (board layer) resets the
 * board if the image hangs.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <string.h>

#include "fdt_walk.h"
#include "rpi5_boot.h"

#define PAGE 0x1000UL
/* The Pi 5 firmware's DTB is some 60 KiB; refuse anything absurd. */
#define MAX_DTB (4UL * 1024 * 1024)

static const char *const wanted[] = { "v3d", "2712-pm", "vc4" };

static uint32_t be32(const uint8_t *p)
{
	return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

/* Map [pa, pa + len) rounded to pages; *off is pa's offset in the first page. */
static const uint8_t *map_blob(uintptr_t pa, size_t len, size_t *off)
{
	uintptr_t base = pa & ~(PAGE - 1);
	mm_reg_t va;

	*off = pa - base;
	device_map(&va, base, ROUND_UP(*off + len, PAGE), K_MEM_CACHE_WB);
	return (const uint8_t *)va + *off;
}

static int wanted_compatible(const uint8_t *v, size_t len)
{
	for (size_t i = 0; i < len; i += strnlen((const char *)v + i, len - i) + 1) {
		for (size_t w = 0; w < ARRAY_SIZE(wanted); w++) {
			if (strstr((const char *)v + i, wanted[w])) {
				return 1;
			}
		}
	}
	return 0;
}

/* Print a string list (compatible) joined with ';'. */
static void print_strlist(const uint8_t *v, size_t len)
{
	for (size_t i = 0; i < len; i += strnlen((const char *)v + i, len - i) + 1) {
		printk("%s%.*s", i ? ";" : "", (int)strnlen((const char *)v + i, len - i),
		       (const char *)v + i);
	}
}

/*
 * The blob passed fdt_walk(), so every token, name and length below is in
 * range. A node is reported at its END_NODE; one that has child nodes is not
 * (the GPU nodes are leaves).
 */
static unsigned int scan(const uint8_t *b)
{
	const uint8_t *st = b + be32(b + 8);
	const uint8_t *strs = b + be32(b + 12);
	size_t end = be32(b + 36);
	size_t p = 0;
	char path[FDT_MAX_PATH + 1];
	size_t plen_at[FDT_MAX_DEPTH + 2] = { 0 };
	unsigned int depth = 0, hits = 0;
	int match = 0, have_reg = 0;
	const uint8_t *reg = NULL;
	size_t reg_len = 0;
	const uint8_t *compat = NULL;
	size_t compat_len = 0;

	path[0] = '\0';
	while (p + 4 <= end) {
		uint32_t tok = be32(st + p);

		p += 4;
		if (tok == 1) { /* BEGIN_NODE */
			const char *name = (const char *)st + p;
			size_t n = strlen(name);
			size_t plen = plen_at[depth];

			if (depth >= 1) {
				if (depth > 1) {
					path[plen++] = '/';
				}
				memcpy(path + plen, name, n);
				plen += n;
			} else {
				plen = 1;
				path[0] = '/';
			}
			path[plen] = '\0';
			depth++;
			plen_at[depth] = plen;
			match = 0;
			have_reg = 0;
			compat = NULL;
			p += (n + 1 + 3) & ~(size_t)3;
		} else if (tok == 2) { /* END_NODE */
			if (match) {
				hits++;
				printk("rpi5_dtb: node %s\n", path);
				printk("rpi5_dtb:   compatible ");
				print_strlist(compat, compat_len);
				printk("\n");
				if (have_reg) {
					printk("rpi5_dtb:   reg");
					for (size_t i = 0; i + 4 <= reg_len; i += 4) {
						printk(" %08x", be32(reg + i));
					}
					printk("\n");
				}
			}
			match = 0;
			have_reg = 0;
			depth--;
		} else if (tok == 3) { /* PROP */
			size_t len = be32(st + p);
			const char *nm = (const char *)strs + be32(st + p + 4);
			const uint8_t *val = st + p + 8;

			if (!strcmp(nm, "compatible")) {
				compat = val;
				compat_len = len;
				match = wanted_compatible(val, len);
			} else if (!strcmp(nm, "reg")) {
				reg = val;
				reg_len = len;
				have_reg = 1;
			}
			p += 8 + ((len + 3) & ~(size_t)3);
		} else if (tok == 9) {
			break;
		}
	}
	return hits;
}

int main(void)
{
	const volatile struct rpi5_boot_state *s;
	struct fdt_trace tr;
	const uint8_t *hdr, *blob;
	size_t off, total;
	uintptr_t dtb;
	int rc;

	printk("rpi5_dtb: build %s\n", RPI5_DTB_BUILD);
	if (!rpi5_boot_valid()) {
		printk("rpi5_dtb: FAIL: no boot record, x0 was not kept\n");
		return 0;
	}
	s = rpi5_boot_state();
	dtb = (uintptr_t)s->x[0];
	printk("rpi5_dtb: x0 0x%llx, EL%llu, entry 0x%llx, mpidr 0x%llx\n",
	       (unsigned long long)s->x[0], (unsigned long long)(s->current_el >> 2),
	       (unsigned long long)s->entry_pc, (unsigned long long)s->mpidr);

	/* The header page, for magic and totalsize. */
	hdr = map_blob(dtb, 40, &off);
	if (be32(hdr) != 0xd00dfeedU) {
		printk("rpi5_dtb: FAIL: no FDT magic at x0 (0x%08x)\n", be32(hdr));
		return 0;
	}
	total = be32(hdr + 4);
	if (total < 40 || total > MAX_DTB) {
		printk("rpi5_dtb: FAIL: totalsize %zu\n", total);
		return 0;
	}

	/* The whole blob. */
	blob = map_blob(dtb, total, &off);
	printk("rpi5_dtb: mapped PA 0x%lx..0x%lx at VA %p\n", dtb & ~(PAGE - 1),
	       (dtb & ~(PAGE - 1)) + ROUND_UP(off + total, PAGE), (const void *)(blob - off));

	rc = fdt_walk(blob, total, &tr);
	printk("rpi5_dtb: totalsize %zu, walk rc %d: %u nodes, %u props, depth %u, path %zu\n", total,
	       rc, tr.nodes, tr.props, tr.max_depth, tr.max_path);
	if (rc != FDT_OK) {
		printk("rpi5_dtb: FAIL: the blob does not walk\n");
		return 0;
	}

	if (scan(blob) == 0) {
		printk("rpi5_dtb: FAIL: no GPU node in the device tree\n");
	} else {
		printk("rpi5_dtb: done\n");
	}
	return 0;
}
