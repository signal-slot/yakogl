/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * snvs_lpgpr - read, write or clear the i.MX8M SNVS LPGPR0..3 (0x30370090..9C)
 * from Linux through /dev/mem, and nothing else.
 *
 *   snvs_lpgpr                      print the four words
 *   snvs_lpgpr write W0 W1 W2 W3    store four words (hex), then print them
 *   snvs_lpgpr clear                store zeros, then print them
 *
 * The four registers are the results channel of a Zephyr image kexec'd onto
 * an i.MX8MP (driver/zephyr/lib/board/imx8mp/gc_result.h): battery-backed, so a
 * record survives the SoC reset that brings Linux back. A board whose kernel
 * has no snvs_lpgpr nvmem driver (Armadillo Base OS) needs this instead, and
 * `dd` cannot do it: device memory has to be mmap'd, read() fails with EFAULT.
 *
 * It maps only the SNVS page and touches only 0x90..0x9C (not the 0x68 alias
 * of LPGPR0), so it cannot be pointed at anything else by mistake.
 * Freestanding like mmio_rd (no libc, static); needs root.
 *
 *   aarch64-linux-gcc -O2 -static -nostdlib -ffreestanding -fno-stack-protector -o snvs_lpgpr snvs_lpgpr.c
 */
typedef unsigned long size_t;

#define SYS_openat 56
#define SYS_close  57
#define SYS_write  64
#define SYS_exit   93
#define SYS_munmap 215
#define SYS_mmap   222

#define AT_FDCWD   (-100)
#define O_RDONLY   0
#define O_RDWR     2
#define O_SYNC     0x101000
#define PROT_READ  1
#define PROT_WRITE 2
#define MAP_SHARED 1
#define PAGE       4096UL

static long sys6(long n, long a, long b, long c, long d, long e, long f)
{
	register long x8 __asm__("x8") = n;
	register long x0 __asm__("x0") = a;
	register long x1 __asm__("x1") = b;
	register long x2 __asm__("x2") = c;
	register long x3 __asm__("x3") = d;
	register long x4 __asm__("x4") = e;
	register long x5 __asm__("x5") = f;

	__asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
	return x0;
}
#define sys(n, ...) sys6(n, __VA_ARGS__)

static size_t slen(const char *s)
{
	size_t n = 0;

	while (s[n]) {
		n++;
	}
	return n;
}

static void put(const char *s)
{
	sys(SYS_write, 1, (long)s, slen(s), 0, 0, 0);
}

static void put_hex(unsigned long v, int digits)
{
	char buf[19] = "0x";
	int n = 2;

	for (int i = (digits - 1) * 4; i >= 0; i -= 4) {
		buf[n++] = "0123456789abcdef"[(v >> i) & 0xf];
	}
	buf[n] = 0;
	put(buf);
}

static void die(const char *what, long err)
{
	put(what);
	put(": error ");
	put_hex(-err, 4);
	put("\n");
	sys(SYS_exit, 1, 0, 0, 0, 0, 0);
}

static unsigned long parse_num(const char *s)
{
	unsigned long v = 0;
	int hex = 0;

	if (s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
		s += 2;
		hex = 1;
	}
	for (; *s; s++) {
		int d = (*s >= '0' && *s <= '9') ? *s - '0'
			: (*s >= 'a' && *s <= 'f') ? *s - 'a' + 10
			: (*s >= 'A' && *s <= 'F') ? *s - 'A' + 10 : -1;
		if (d < 0 || (!hex && d > 9)) {
			break;
		}
		v = v * (hex ? 16 : 10) + d;
	}
	return v;
}

#define SNVS_PAGE  0x30370000UL
#define LPGPR_OFF  0x90UL

static int streq(const char *a, const char *b)
{
	while (*a && *a == *b) {
		a++;
		b++;
	}
	return *a == *b;
}

int main(int argc, char **argv)
{
	int write = argc == 6 && streq(argv[1], "write");
	int clear = argc == 2 && streq(argv[1], "clear");
	unsigned int val[4] = { 0, 0, 0, 0 };

	if (argc != 1 && !write && !clear) {
		put("usage: snvs_lpgpr [write W0 W1 W2 W3 | clear]\n");
		return 2;
	}
	if (write) {
		for (int i = 0; i < 4; i++) {
			val[i] = (unsigned int)parse_num(argv[2 + i]);
		}
	}
	long fd = sys(SYS_openat, AT_FDCWD, (long)"/dev/mem",
		      ((write || clear) ? O_RDWR : O_RDONLY) | O_SYNC, 0, 0, 0);
	if (fd < 0) {
		die("/dev/mem", fd);
	}
	long map = sys(SYS_mmap, 0, PAGE, PROT_READ | ((write || clear) ? PROT_WRITE : 0),
		       MAP_SHARED, fd, SNVS_PAGE);
	if (map < 0 && map > -4096) {
		die("mmap", map);
	}
	volatile unsigned int *r = (volatile unsigned int *)(map + LPGPR_OFF);

	if (write || clear) {
		/* Words 1..3 first and word 0 last, the order gc_result.c keeps,
		 * so a reader never sees a magic over half-written words. */
		for (int i = 3; i >= 0; i--) {
			r[i] = val[i];
			__asm__ volatile("dsb sy" ::: "memory");
		}
	}
	for (int i = 0; i < 4; i++) {
		put_hex(SNVS_PAGE + LPGPR_OFF + i * 4, 8);
		put(": ");
		put_hex(r[i], 8);
		put("\n");
	}
	sys(SYS_munmap, map, PAGE, 0, 0, 0, 0);
	sys(SYS_close, fd, 0, 0, 0, 0, 0);
	return 0;
}

/* Process entry: argc and argv sit on the initial stack. */
void start_c(long *sp)
{
	int rc = main((int)sp[0], (char **)(sp + 1));

	sys(SYS_exit, rc, 0, 0, 0, 0, 0);
}

__asm__(".global _start\n"
	"_start:\n"
	"	mov x0, sp\n"
	"	bl start_c\n");
