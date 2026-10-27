/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Runtime of the generated entry-point interposer (see gen_wrap.py). */
#include <stdio.h>
#include <string.h>
#include "hh_wrap.h"

uint64_t hh_calls[4096];
int hh_cur = -1;

/* Is the application inside a call that creates or re-creates object
 * storage (buffers, textures, renderbuffers, surfaces)? HH_NOCPU gives only
 * those allocations no CPU view: the PowerVR backend needs CPU-mapped arenas
 * and programs, the GL core has fallbacks for its object storage. */
int hh_storage_call(void)
{
	static const char *const pre[] = {
		"glBufferData", "glBufferSubData", "glMapBufferRange", "glUnmapBuffer", "glFlushMappedBufferRange",
		"glTexImage", "glTexSubImage", "glTexStorage", "glCompressedTex", "glCopyTex", "glGenerateMipmap",
		"glRenderbufferStorage", "glEGLImageTarget", "glBlitFramebuffer", "eglCreatePbufferSurface",
		"eglCreateWindowSurface", "eglCreatePlatformWindowSurface", "eglCreateImage",
	};

	if (hh_cur < 0)
		return 0;
	for (unsigned i = 0; i < sizeof(pre) / sizeof(pre[0]); i++)
		if (!strncmp(hh_names[hh_cur], pre[i], strlen(pre[i])))
			return 1;
	return 0;
}
unsigned char hh_probe_buf[1u << 20];
unsigned char hh_probe_a[1u << 20];

size_t hh_probe_extent(size_t esz)
{
	size_t last = 0;

	/* a byte written by the call reads the same after both runs; one left
	 * alone reads 0xA5 then 0x5A */
	for (size_t i = 0; i < HH_PROBE_BYTES; i++)
		if (!(hh_probe_a[i] == 0xA5 && hh_probe_buf[i] == 0x5A))
			last = i + 1;
	return (last + esz - 1) / esz;
}

void hh_vals(const char *kind, const void *p, size_t n)
{
	const unsigned char *b = p;

	if (!p) {
		hh_printf("-");
		return;
	}
	hh_printf("[%zu]", n);
	for (size_t i = 0; i < n && i < 256; i++) {
		if (!strcmp(kind, "u8")) {
			hh_printf("%s%u", i ? "," : "", b[i]);
		} else if (!strcmp(kind, "i32") || !strcmp(kind, "x32")) {
			int32_t v;

			memcpy(&v, b + 4 * i, 4);
			hh_printf(kind[0] == 'i' ? "%s%d" : "%s%x", i ? "," : "", kind[0] == 'i' ? v : (int32_t)(uint32_t)v);
		} else if (!strcmp(kind, "i64")) {
			int64_t v;

			memcpy(&v, b + 8 * i, 8);
			hh_printf("%s%lld", i ? "," : "", (long long)v);
		} else {
			uint64_t v;

			/* application pointers (client arrays) move with the link
			 * layout; small values are buffer offsets */
			memcpy(&v, b + 8 * i, 8);
			if (v < (1ull << 32))
				hh_printf("%s%llu", i ? "," : "", (unsigned long long)v);
			else
				hh_printf("%sP", i ? "," : "");
		}
	}
	if (n > 256)
		hh_printf(",...#%016llx", (unsigned long long)hh_hash(p, n * (kind[0] == 'u' ? 1 : kind[1] == '6' || kind[0] == 'p' ? 8 : 4)));
}

static size_t esc(char *o, size_t cap, const char *s, size_t max)
{
	size_t k = 0;

	for (size_t i = 0; i < max && s[i] && k + 5 < cap; i++) {
		unsigned char c = (unsigned char)s[i];

		if (c >= 0x20 && c < 0x7f && c != '\\')
			o[k++] = (char)c;
		else
			k += (size_t)snprintf(o + k, cap - k, "\\x%02x", c);
	}
	o[k] = 0;
	return k;
}

void hh_str(const char *s, size_t max)
{
	static char buf[16384];

	if (!s) {
		hh_printf("(null)");
		return;
	}
	esc(buf, sizeof(buf), s, max);
	hh_printf("\"%s\"", buf);
}

const char *hh_cs(const char *s)
{
	static char bufs[4][256];
	static unsigned k;
	char *b = bufs[k++ % 4];

	if (!s)
		return "(null)";
	b[0] = '"';
	esc(b + 1, sizeof(bufs[0]) - 2, s, 96);
	strcat(b, "\"");
	return b;
}

unsigned hh_hid(void *p)
{
	static void *tab[4096];
	static unsigned n;

	if (!p)
		return 0;
	for (unsigned i = 0; i < n; i++)
		if (tab[i] == p)
			return i + 1;
	if (n < 4096)
		tab[n++] = p;
	return n;
}

static int pack_align = 4;

void hh_pixelstore(unsigned pname, int param)
{
	if (pname == 0x0D05 /* GL_PACK_ALIGNMENT */ && (param == 1 || param == 2 || param == 4 || param == 8))
		pack_align = param;
}

void hh_readpixels(int w, int h, unsigned format, unsigned type, void *pixels)
{
	size_t comps, bpp, row, total;

	switch (format) {
	case 0x1906: case 0x1909: case 0x1903: case 0x8D94: case 0x1902: case 0x1901: comps = 1; break; /* A L R R_INT D S */
	case 0x190A: case 0x8227: case 0x8228: comps = 2; break;                                       /* LA RG RG_INT */
	case 0x1907: case 0x8D98: comps = 3; break;                                                    /* RGB RGB_INT */
	default: comps = 4; break;                                                                     /* RGBA BGRA ... */
	}
	switch (type) {
	case 0x1401: case 0x1400: bpp = comps; break;                      /* (U)BYTE */
	case 0x8363: case 0x8033: case 0x8034: bpp = 2; break;             /* 565 4444 5551 */
	case 0x1403: case 0x1402: case 0x140B: case 0x8D61: bpp = 2 * comps; break;
	case 0x8368: case 0x8C3B: case 0x8C3E: bpp = 4; break;             /* 2_10_10_10 10F_11F_11F 5_9_9_9 */
	default: bpp = 4 * comps; break;                                   /* (U)INT FLOAT */
	}
	if (!pixels || w <= 0 || h <= 0 || (uint64_t)w * (uint64_t)h > (1u << 24)) {
		hh_printf(" pixels=-");
		return;
	}
	row = ((size_t)w * bpp + (size_t)pack_align - 1) / (size_t)pack_align * (size_t)pack_align;
	total = row * (size_t)(h - 1) + (size_t)w * bpp;
	hh_printf(" pixels[%zu]=#%016llx", total, (unsigned long long)hh_hash(pixels, total));
}
