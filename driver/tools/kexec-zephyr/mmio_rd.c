/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mmio_rd - print 32-bit words of device memory from Linux through /dev/mem.
 *
 *   mmio_rd <physical address (hex)> [count]
 *   mmio_rd <physical address (hex)> = <value (hex)>     (write one word)
 *   mmio_rd <physical address (hex)> <count> raw            (bytes to stdout)
 *
 * Freestanding like kexec_zephyr (no libc, static): the board has neither
 * devmem nor python, and dd cannot read device memory (EFAULT). It mmaps
 * the page, so it works with CONFIG_STRICT_DEVMEM, which only forbids RAM.
 * Needs root (a privileged container is enough). Used to read the result
 * registers a Zephyr sample leaves in the SNVS after it resets the SoC.
 *
 *   aarch64-linux-gcc -O2 -static -nostdlib -ffreestanding -fno-stack-protector -o mmio_rd mmio_rd.c
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

int main(int argc, char **argv)
{
	if (argc < 2) {
		put("usage: mmio_rd <physical address (hex)> [count]\n");
		return 1;
	}
	unsigned long addr = parse_num(argv[1]);
	int write = argc > 3 && argv[2][0] == '=' && argv[2][1] == 0;
	int raw = argc > 3 && argv[3][0] == 'r' && argv[3][1] == 'a' && argv[3][2] == 'w' && argv[3][3] == 0;
	unsigned long count = write ? 1 : argc > 2 ? parse_num(argv[2]) : 1;
	unsigned long base = addr & ~(PAGE - 1);
	unsigned long len = ((addr + count * 4 - base) + PAGE - 1) & ~(PAGE - 1);

	long fd = sys(SYS_openat, AT_FDCWD, (long)"/dev/mem", (write ? O_RDWR : O_RDONLY) | O_SYNC, 0, 0, 0);
	if (fd < 0) {
		die("/dev/mem", fd);
	}
	long map = sys(SYS_mmap, 0, len, PROT_READ | (write ? PROT_WRITE : 0), MAP_SHARED, fd, base);
	if (map < 0 && map > -4096) {
		die("mmap", map);
	}
	if (write) {
		*(volatile unsigned int *)(map + (addr - base)) = (unsigned int)parse_num(argv[3]);
	}
	if (raw) {
		/* Device memory wants aligned word accesses: copy through a buffer. */
		static unsigned int buf[1024];

		for (unsigned long done = 0; done < count; done += 1024) {
			unsigned long n = count - done < 1024 ? count - done : 1024;

			for (unsigned long i = 0; i < n; i++) {
				buf[i] = *(volatile unsigned int *)(map + (addr - base) + (done + i) * 4);
			}
			sys(SYS_write, 1, (long)buf, n * 4, 0, 0, 0);
		}
		count = 0;
	}
	for (unsigned long i = 0; i < count; i++) {
		volatile unsigned int *p = (volatile unsigned int *)(map + (addr - base) + i * 4);

		put_hex(addr + i * 4, 8);
		put(": ");
		put_hex(*p, 8);
		put("\n");
	}
	sys(SYS_munmap, map, len, 0, 0, 0, 0);
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
