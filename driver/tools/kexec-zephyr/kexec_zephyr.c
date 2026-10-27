// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * kexec_zephyr - boot a Zephyr AArch64 image from Linux without kexec-tools.
 *
 * The Torizon kernel on the Verdin AM62P has CONFIG_KEXEC (the kexec_load
 * system call) but ships no kexec binary. A Zephyr image is a flat binary
 * linked at a fixed physical address (0x82000000 on the SafeUI board), so
 * one segment placed at that address with the entry point at its start is
 * all the kernel needs: the arm64 kexec path stops the other CPUs, turns the
 * MMU and caches off and jumps to the entry with x0..x3 = 0 (the same state
 * U-Boot's "go" gives).
 *
 *   kexec_zephyr <zephyr.bin> [load address (hex), default 0x82000000] [file@addr ...]
 *                [--crash] [--no-watchdog]
 *
 * On the AM62P the jump is refused unless the MAIN RTI watchdog is running.
 * A Zephyr image there feeds the watchdog Linux left armed, for a bounded
 * time, and has no other way back: the SafeUI SoC code feeds a running RTI
 * and otherwise logs "RTI watchdog not running, nothing to feed", so an image
 * that then hangs holds the board until someone cuts the power (2026-09-23:
 * a Qt Quick 3D demo jumped ~20 s into Linux's boot, hung, and was never
 * reset). --no-watchdog skips the check for a deliberate experiment. On any
 * other SoC nothing is checked, and the loader says so.
 *
 * --crash takes the kernel's crash route (see crash_jump) so that the display
 * Linux was driving stays on; it needs a crashkernel= reservation that holds
 * the image.
 *
 * Extra "file@addr" arguments are placed at the given physical addresses as
 * further segments before the jump: e.g. the SafeUI MCU R5F firmware images
 * (r5_ddr.bin@0x9b900000 r5_tcm.bin@0x9b880000) that U-Boot's bootcmd would
 * load, so the Supervisor can start the R5F. Up to 15 extra segments.
 *
 * Freestanding (raw system calls, no libc) so that any aarch64 GCC builds
 * it and the binary runs on any aarch64 Linux:
 *   aarch64-linux-gcc -O2 -static -nostdlib -ffreestanding -o kexec_zephyr kexec_zephyr.c
 *
 * Unload the stock GPU driver first (rmmod pvrsrvkm) so the GPU is idle and
 * powered down when Zephyr's driver takes it over.
 */
#include <stddef.h>
#include <stdint.h>

#define SYS_openat     56
#define SYS_close      57
#define SYS_rt_sigaction 134
#define SYS_socket     198
#define SYS_sendto     206
#define SYS_nanosleep  101
#define SYS_read       63
#define SYS_write      64
#define SYS_exit       93
#define SYS_sync       81
#define SYS_reboot     142
#define SYS_kexec_load 104
#define SYS_mmap       222

#define AT_FDCWD          (-100)
#define O_RDONLY          0
#define O_WRONLY          1
#define PROT_READ         1
#define PROT_WRITE        2
#define MAP_PRIVATE       2
#define MAP_ANONYMOUS     0x20
#define KEXEC_ARCH_AARCH64 (183 << 16)
#define KEXEC_ON_CRASH     1
#define LINUX_REBOOT_MAGIC1    0xfee1dead
#define LINUX_REBOOT_MAGIC2    672274793
#define LINUX_REBOOT_CMD_KEXEC 0x45584543

struct kexec_segment {
	const void *buf;
	size_t bufsz;
	const void *mem;
	size_t memsz;
};

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
	sys(SYS_write, 2, (long)s, slen(s), 0, 0, 0);
}

static void put_hex(unsigned long v)
{
	char buf[19] = "0x";
	int i, n = 2;

	for (i = 60; i >= 0; i -= 4) {
		int d = (v >> i) & 0xf;

		if (n > 2 || d || i == 0) {
			buf[n++] = "0123456789abcdef"[d];
		}
	}
	buf[n] = 0;
	put(buf);
}

static void die(const char *what, long err)
{
	put(what);
	put(": error ");
	put_hex(-err);
	put("\n");
	sys(SYS_exit, 1, 0, 0, 0, 0, 0);
}

static unsigned long parse_hex(const char *s)
{
	unsigned long v = 0;

	if (s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
		s += 2;
	}
	for (; *s; s++) {
		int d = (*s >= '0' && *s <= '9') ? *s - '0'
			: (*s >= 'a' && *s <= 'f') ? *s - 'a' + 10
			: (*s >= 'A' && *s <= 'F') ? *s - 'A' + 10 : -1;
		if (d < 0) {
			break;
		}
		v = (v << 4) | d;
	}
	return v;
}

/* Each file is read into its own anonymous mapping (Zephyr images and the
 * R5F blobs are a few MiB; 64 MiB is reserved per file). */
#define IMG_MAX (64UL << 20)
#define MAX_SEGMENTS 16

/* Read a small text file (a sysfs attribute) into buf; returns its length. */
static long read_small(const char *path, char *buf, long max)
{
	long fd = sys(SYS_openat, AT_FDCWD, (long)path, O_RDONLY, 0, 0, 0);
	long n;

	if (fd < 0) {
		return fd;
	}
	n = sys(SYS_read, fd, (long)buf, max - 1, 0, 0, 0);
	sys(SYS_close, fd, 0, 0, 0, 0, 0);
	if (n < 0) {
		return n;
	}
	buf[n] = 0;
	return n;
}

/* The AM62P's MAIN RTI0 (k3-am62-main.dtsi main_rti0: watchdog@e000000) and
 * the DWDCTRL value that means "digital watchdog enabled". */
#define AM62P_RTI0_BASE      0x0e000000UL
#define RTI_DWDCTRL          0x90
#define RTI_DWDCTRL_ENABLED  0xa98559daUL
#define O_RDWR_SYNC          (2 | 0x101000)   /* O_RDWR | O_SYNC */
#define MAP_SHARED           1

static int has_bytes(const char *buf, long n, const char *w)
{
	long k = (long)slen(w);

	for (long i = 0; i + k <= n; i++) {
		long j = 0;

		while (j < k && buf[i + j] == w[j]) {
			j++;
		}
		if (j == k) {
			return 1;
		}
	}
	return 0;
}

/* 0: safe to jump (or not an AM62P); 1: refuse. */
static int watchdog_gate(int skip)
{
	char compat[512];
	long n = read_small("/proc/device-tree/compatible", compat, sizeof(compat));
	long fd, map;
	unsigned long v;

	if (n <= 0 || !has_bytes(compat, n, "ti,am62p5")) {
		put("watchdog: not an AM62P, no watchdog check on this SoC\n");
		return 0;
	}
	if (skip) {
		put("watchdog: --no-watchdog: jumping without checking the RTI (a hang holds the board)\n");
		return 0;
	}
	fd = sys(SYS_openat, AT_FDCWD, (long)"/dev/mem", O_RDWR_SYNC, 0, 0, 0);
	if (fd < 0) {
		die("watchdog: /dev/mem (needed to check the RTI; --no-watchdog skips it)", fd);
	}
	map = sys(SYS_mmap, 0, 4096, PROT_READ, MAP_SHARED, fd, AM62P_RTI0_BASE);
	sys(SYS_close, fd, 0, 0, 0, 0, 0);
	if (map < 0 && map > -4096) {
		die("watchdog: mmap of the RTI", map);
	}
	v = *(volatile unsigned int *)(map + RTI_DWDCTRL);
	put("watchdog: RTI0 DWDCTRL = ");
	put_hex(v);
	if (v != RTI_DWDCTRL_ENABLED) {
		put(": not running. Refusing the jump: the image would have no watchdog.\n"
		    "watchdog: wait for Linux to arm it (systemctl is-system-running --wait),\n"
		    "watchdog: or pass --no-watchdog for a deliberate experiment.\n");
		return 1;
	}
	put(": running\n");
	return 0;
}

static void ignore_signal(int signum)
{
	struct {
		void *handler;
		unsigned long flags;
		void *restorer;
		unsigned long mask;
	} act = { (void *)1 /* SIG_IGN */, 0, 0, 0 };

	sys(SYS_rt_sigaction, signum, (long)&act, 0, 8, 0, 0);
}

/* ip link set <name> down, through rtnetlink. */
static int link_down(const char *name)
{
	char path[64], num[16];
	long i = 0, idx = 0, fd, n;
	struct {
		unsigned int len;
		unsigned short type, flags;
		unsigned int seq, pid;
		unsigned char family, pad;
		unsigned short dev_type;
		int index;
		unsigned int if_flags, change;
	} req;
	struct {
		unsigned short family, pad;
		unsigned int pid, groups;
	} addr = { 16 /* AF_NETLINK */, 0, 0, 0 };

	for (const char *s = "/sys/class/net/"; *s; s++) {
		path[i++] = *s;
	}
	for (const char *s = name; *s; s++) {
		path[i++] = *s;
	}
	for (const char *s = "/ifindex"; *s; s++) {
		path[i++] = *s;
	}
	path[i] = 0;
	if (read_small(path, num, sizeof(num)) <= 0) {
		return -1;   /* no such interface */
	}
	for (i = 0; num[i] >= '0' && num[i] <= '9'; i++) {
		idx = idx * 10 + (num[i] - '0');
	}
	fd = sys(SYS_socket, 16 /* AF_NETLINK */, 3 /* SOCK_RAW */, 0 /* NETLINK_ROUTE */, 0, 0, 0);
	if (fd < 0) {
		return (int)fd;
	}
	req.len = sizeof(req);
	req.type = 16;      /* RTM_NEWLINK */
	req.flags = 1 | 4;  /* NLM_F_REQUEST | NLM_F_ACK */
	req.seq = 1;
	req.pid = 0;
	req.family = 0;
	req.pad = 0;
	req.dev_type = 0;
	req.index = (int)idx;
	req.if_flags = 0;   /* IFF_UP cleared */
	req.change = 1;     /* IFF_UP */
	n = sys(SYS_sendto, fd, (long)&req, sizeof(req), 0, (long)&addr, sizeof(addr));
	sys(SYS_close, fd, 0, 0, 0, 0, 0);
	return n < 0 ? (int)n : 0;
}

/* Bus masters Linux leaves running are not stopped by the kexec path, and
 * their DMA keeps writing into RAM that the new image now owns. The CPSW
 * Ethernet controller is the one that bit: its receive DMA kept dropping
 * every broadcast frame on the LAN into its old buffers at 0x100 of a page,
 * which by then were Zephyr's rodata (2026-09-16). Taking the interface
 * down runs the driver's stop path, which tears the DMA channels down
 * (unbinding the driver instead oopses in free_netdev on Torizon 7.6).
 * The ssh session that started us dies with the link, so its signals are
 * ignored from here on; the next step is the jump anyway. */
static void quiesce_bus_masters(void);

/* Write a short string to a sysfs/procfs file; 0 on success. */
static int write_file(const char *path, const char *text)
{
	long fd = sys(SYS_openat, AT_FDCWD, (long)path, O_WRONLY, 0, 0, 0);
	long n;

	if (fd < 0) {
		return (int)fd;
	}
	n = sys(SYS_write, fd, (long)text, slen(text), 0, 0, 0);
	sys(SYS_close, fd, 0, 0, 0, 0, 0);
	return n < 0 ? (int)n : 0;
}

/*
 * The crash route. The normal kexec (reboot(KEXEC)) first runs
 * device_shutdown(), and the display drivers turn the panel off in theirs, so
 * an image that wants the picture Linux was showing arrives to a dark screen.
 * A panic takes the crash route instead (__crash_kexec: no device shutdown,
 * the other CPUs are only parked), which needs the image loaded with
 * KEXEC_ON_CRASH inside the crashkernel= reservation. The secondary CPUs are
 * offlined first so they go through PSCI CPU_OFF and the image can CPU_ON
 * them, rather than being left spinning in the old kernel's park loop.
 */
static void crash_jump(void)
{
	static const char *const cpus[] = {
		"/sys/devices/system/cpu/cpu1/online", "/sys/devices/system/cpu/cpu2/online",
		"/sys/devices/system/cpu/cpu3/online", "/sys/devices/system/cpu/cpu4/online",
		"/sys/devices/system/cpu/cpu5/online", "/sys/devices/system/cpu/cpu6/online",
		"/sys/devices/system/cpu/cpu7/online",
	};

	for (unsigned long i = 0; i < sizeof(cpus) / sizeof(cpus[0]); i++) {
		(void)write_file(cpus[i], "0");
	}
	(void)write_file("/proc/sys/kernel/sysrq", "1");
	sys(SYS_sync, 0, 0, 0, 0, 0, 0);
	quiesce_bus_masters();
	(void)write_file("/proc/sysrq-trigger", "c");
}

static void quiesce_bus_masters(void)
{
	static const char *const ifaces[] = { "ethernet0", "eth0", "end0" };
	struct {
		long sec, nsec;
	} ts = { 0, 300 * 1000 * 1000 };

	ignore_signal(1);    /* SIGHUP */
	ignore_signal(13);   /* SIGPIPE */
	ignore_signal(15);   /* SIGTERM */
	for (unsigned i = 0; i < sizeof(ifaces) / sizeof(ifaces[0]); i++) {
		int rc = link_down(ifaces[i]);

		if (rc == 0) {
			put(ifaces[i]);
			put(": link down, DMA stopped\n");
		}
	}
	sys(SYS_nanosleep, (long)&ts, 0, 0, 0, 0, 0);   /* let the stop path finish */
}

static long read_file(const char *path, char **out)
{
	long fd, n, size = 0;
	char *img;

	fd = sys(SYS_openat, AT_FDCWD, (long)path, O_RDONLY, 0, 0, 0);
	if (fd < 0) {
		die(path, fd);
	}
	img = (char *)sys(SYS_mmap, 0, IMG_MAX, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if ((unsigned long)img > -4096UL) {
		die("mmap", (long)img);
	}
	while ((n = sys(SYS_read, fd, (long)(img + size), IMG_MAX - size, 0, 0, 0)) > 0) {
		size += n;
	}
	if (n < 0) {
		die("read", n);
	}
	*out = img;
	return size;
}

int main(int argc, char **argv)
{
	unsigned long load = argc > 2 ? parse_hex(argv[2]) : 0x82000000UL;
	long n, size;
	char *img;
	struct kexec_segment seg[MAX_SEGMENTS];
	int nseg = 0;
	int crash = 0, no_wdt = 0;

	if (argc < 2) {
		put("usage: kexec_zephyr <zephyr.bin> [load address] [file@addr ...] [--crash] [--no-watchdog]\n");
		return 2;
	}
	/* trailing flags, in either order */
	while (argc > 2 && argv[argc - 1][0] == '-' && argv[argc - 1][1] == '-') {
		if (argv[argc - 1][2] == 'c') {
			crash = 1;
		} else if (argv[argc - 1][2] == 'n') {
			no_wdt = 1;
		} else {
			put(argv[argc - 1]);
			put(": unknown flag\n");
			return 2;
		}
		argc--;
	}
	if (watchdog_gate(no_wdt)) {
		return 3;
	}
	load = argc > 2 ? parse_hex(argv[2]) : 0x82000000UL;
	size = read_file(argv[1], &img);
	if (size < 64 || img[56] != 'A' || img[57] != 'R' || img[58] != 'M' || img[59] != 0x64) {
		put(argv[1]);
		put(": not an AArch64 Image (CONFIG_AARCH64_IMAGE_HEADER)\n");
		return 1;
	}
	/* The segment covers the image's whole RAM footprint, not just the file:
	 * the AArch64 header's image_size (Zephyr: _flash_used) includes .bss and
	 * .noinit. The kernel zero-fills the tail and cleans + invalidates the
	 * data cache over the entire memsz before it jumps. Without that, lines
	 * Linux left dirty in the L2 for those addresses come back later: Zephyr
	 * zeroes .bss with the MMU (and so the cache) off, then enables the cache,
	 * and the stale dirty lines resurface as garbage in its page tables --
	 * a random "entry already in use" panic at boot, about one run in three. */
	unsigned long image_size = *(unsigned long *)(img + 16);
	unsigned long memsz = image_size > (unsigned long)size ? image_size : (unsigned long)size;

	seg[nseg].buf = img;
	seg[nseg].bufsz = size;
	seg[nseg].mem = (void *)load;
	seg[nseg].memsz = (memsz + 4095) & ~4095UL;
	nseg++;

	for (int i = 3; i < argc && nseg < MAX_SEGMENTS; i++) {
		char *at = argv[i];
		unsigned long addr;
		long sz;
		char *blob;

		while (*at && *at != '@') {
			at++;
		}
		if (*at != '@') {
			put(argv[i]);
			put(": expected file@addr\n");
			return 2;
		}
		*at = 0;
		addr = parse_hex(at + 1);
		sz = read_file(argv[i], &blob);
		seg[nseg].buf = blob;
		seg[nseg].bufsz = sz;
		seg[nseg].mem = (void *)addr;
		seg[nseg].memsz = (sz + 4095) & ~4095UL;
		nseg++;
		put("segment ");
		put(argv[i]);
		put(": ");
		put_hex(sz);
		put(" bytes at ");
		put_hex(addr);
		put("\n");
	}

	n = sys(SYS_kexec_load, load, nseg, (long)seg,
		KEXEC_ARCH_AARCH64 | (crash ? KEXEC_ON_CRASH : 0), 0, 0);
	if (n < 0) {
		die("kexec_load", n);
	}
	put("loaded ");
	put_hex(size);
	put(" bytes at ");
	put_hex(load);
	put(" (segment ");
	put_hex(seg[0].memsz);
	put(" bytes: the image's RAM footprint, zeroed and cache-cleaned), jumping to it\n");
	/* The kexec path does not unmount or sync: without this, files written
	 * just before (the image itself, copied over scp) are still in the page
	 * cache and come back damaged after the reset. */
	if (crash) {
		crash_jump();
		die("sysrq crash did not jump (crashkernel= reserved? image inside it?)", -1);
	}
	sys(SYS_sync, 0, 0, 0, 0, 0, 0);
	quiesce_bus_masters();
	n = sys(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2, LINUX_REBOOT_CMD_KEXEC, 0, 0, 0);
	die("reboot(KEXEC)", n);
	return 1;
}

/* Process entry: argc and argv sit on the initial stack. */
__asm__(".global _start\n"
	"_start:\n"
	"  ldr x0, [sp]\n"
	"  add x1, sp, #8\n"
	"  bl main\n"
	"  mov x8, #93\n"
	"  svc #0\n");
