// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * crash_irq.h - two steps of kexec_zephyr's crash route before the panic:
 * unbind the PMIC's power key (crash_pwrkey_unbind), and, last before the
 * links go down, refuse when /proc/interrupts still has an interrupt the
 * panic may hang on (crash_irq_check). crash_refuse re-binds the power key
 * (crash_pwrkey_rebind). driver/docs/nanopc-t6-display.md,
 * section 2(a), has the measurements.
 *
 * Why. On the crash route machine_crash_shutdown masks and shuts down every
 * started interrupt (linux v6.18 arch/arm64/kernel/machine_kexec.c:213-225,
 * kernel/irq/kexec.c), and that takes each interrupt's chip bus lock
 * (kernel/irq/manage.c:2689-2711). The NanoPC-T6's PMIC, an RK806 on SPI,
 * is a regmap-irq chip: its bus lock is a mutex and its unlock writes the
 * PMIC's registers over SPI (drivers/base/regmap/regmap-irq.c:76-207), which
 * waits for the SPI controller's interrupt, shut down earlier in the same
 * loop, on the one CPU left with interrupts off. The panic then sleeps for
 * ever and Linux's watchdog resets the board. Two interrupts of the PMIC are
 * started: the power key's (driver rk805-pwrkey). On 2026-11-15 the jump
 * hung with that driver bound and went through twice with it unbound, the
 * table then having no row on chip rk806.
 *
 * User space cannot ask a chip whether its bus lock sleeps. So the check
 * lets an interrupt through only on a chip the crash route went through on
 * the board (crash_irq_chips) and refuses any other.
 *
 * Freestanding, like kexec_zephyr.c. The includer defines the five
 * functions declared below; crash_irq_test.c gives them made-up /sys and
 * /proc files to test these steps on the host (crash_irq_selftest.sh).
 */
#ifndef CRASH_IRQ_H
#define CRASH_IRQ_H

static void put(const char *s);
static void put_hex(unsigned long v);
/* The whole file, NUL-terminated, at most max - 1 bytes; < 0: -errno. */
static long read_small(const char *path, char *buf, long max);
/* 0, or -errno. */
static int write_file(const char *path, const char *text);
/* The names in dir, each ended by a NUL; their total length, or -errno. */
static long list_dir(const char *dir, char *names, long max);

#define CRASH_PWRKEY_DRIVER "rk805-pwrkey"
#define CRASH_PWRKEY_DIR    "/sys/bus/platform/drivers/" CRASH_PWRKEY_DRIVER

/* What crash_pwrkey_unbind took off the driver, for the re-bind. */
static char crash_pwrkey_names[1024];
static long crash_pwrkey_len;

static int crash_streq(const char *a, const char *b)
{
	while (*a && *a == *b) {
		a++;
		b++;
	}
	return *a == *b;
}

static int crash_starts(const char *s, long n, const char *w)
{
	long i = 0;

	for (; w[i]; i++) {
		if (i >= n || s[i] != w[i]) {
			return 0;
		}
	}
	return 1;
}

/*
 * Every device bound to rk805-pwrkey, unbound. The names are listed, never
 * written down here: the platform device's number changes from boot to boot
 * (rk805-pwrkey.3.auto, rk805-pwrkey.4.auto). Beside the devices the
 * directory holds the driver's own entries (bind, unbind, uevent, and
 * module when it is a module), which are not devices. No directory or no
 * device is not an error. A failed unbind is printed; the interrupt check
 * after it is what decides.
 */
static void crash_pwrkey_unbind(void)
{
	static char names[1024];
	long n = list_dir(CRASH_PWRKEY_DIR, names, sizeof(names));
	int devices = 0;

	if (n == -2) {
		put("crash: no " CRASH_PWRKEY_DIR ": no power key to unbind\n");
		return;
	}
	if (n < 0) {
		put("crash: " CRASH_PWRKEY_DIR " cannot be listed: error ");
		put_hex(-n);
		put("\n");
		return;
	}
	for (long off = 0; off < n;) {
		const char *name = names + off;
		long len = 0;
		int rc;

		while (off + len < n && name[len]) {
			len++;
		}
		off += len + 1;
		if (name[0] == '.' || crash_streq(name, "bind") || crash_streq(name, "unbind") ||
		    crash_streq(name, "uevent") || crash_streq(name, "module")) {
			continue;
		}
		devices++;
		rc = write_file(CRASH_PWRKEY_DIR "/unbind", name);
		put("crash: ");
		put(name);
		if (rc != 0) {
			put(": unbind from " CRASH_PWRKEY_DRIVER " FAILED: error ");
			put_hex(-rc);
			put("\n");
			continue;
		}
		put(" unbound from " CRASH_PWRKEY_DRIVER "\n");
		if (crash_pwrkey_len + len + 1 <= (long)sizeof(crash_pwrkey_names)) {
			for (long i = 0; i <= len; i++) {
				crash_pwrkey_names[crash_pwrkey_len + i] = name[i];
			}
			crash_pwrkey_len += len + 1;
		} else {
			put("crash: ");
			put(name);
			put(": no room to note it for a re-bind\n");
		}
	}
	if (devices == 0) {
		put("crash: no device bound to " CRASH_PWRKEY_DRIVER "\n");
	}
}

/* Binds back what crash_pwrkey_unbind unbound; the number that failed. */
static int crash_pwrkey_rebind(void)
{
	int failed = 0;

	for (long off = 0; off < crash_pwrkey_len;) {
		const char *name = crash_pwrkey_names + off;
		long len = 0;
		int rc;

		while (name[len]) {
			len++;
		}
		off += len + 1;
		rc = write_file(CRASH_PWRKEY_DIR "/bind", name);
		put("crash: ");
		put(name);
		if (rc != 0) {
			put(": bind to " CRASH_PWRKEY_DRIVER " FAILED: error ");
			put_hex(-rc);
			put("; the power key stays unbound\n");
			failed++;
		} else {
			put(" bound to " CRASH_PWRKEY_DRIVER " again\n");
		}
	}
	crash_pwrkey_len = 0;
	return failed;
}

/*
 * The chips the crash route went through on the NanoPC-T6 (2026-11-15,
 * Linux 6.18.55: every chip of its /proc/interrupts but rk806): the GIC,
 * its PPI partition, the ITS's PCI MSI and MSI-X domains (one per PCI
 * device, so by prefix) and the SoC's GPIO banks.
 */
static const struct {
	const char *name;
	int prefix;
} crash_irq_chips[] = {
	{ "GICv3", 0 }, { "GICv3-23", 0 }, { "ITS-PCI-MSI-", 1 },
	{ "ITS-PCI-MSIX-", 1 }, { "rockchip_gpio_irq", 0 },
};

static int crash_irq_chip_measured(const char *chip, long n)
{
	for (unsigned long i = 0; i < sizeof(crash_irq_chips) / sizeof(crash_irq_chips[0]); i++) {
		const char *w = crash_irq_chips[i].name;
		long k = 0;

		while (w[k]) {
			k++;
		}
		if (crash_starts(chip, n, w) && (crash_irq_chips[i].prefix ? n > k : n == k)) {
			return 1;
		}
	}
	return 0;
}

static int crash_space(char c)
{
	return c == ' ' || c == '\t';
}

/* The token at *p (up to a space or the line's end); *p moves past it. */
static long crash_token(const char **p, const char *end, const char **tok)
{
	const char *s = *p;

	while (s < end && crash_space(*s)) {
		s++;
	}
	*tok = s;
	while (s < end && !crash_space(*s)) {
		s++;
	}
	*p = s;
	return s - *tok;
}

static int crash_digits(const char *s, long n)
{
	if (n == 0) {
		return 0;
	}
	for (long i = 0; i < n; i++) {
		if (s[i] < '0' || s[i] > '9') {
			return 0;
		}
	}
	return 1;
}

/* The row, its runs of spaces made one, in quotes. */
static void crash_put_row(const char *s, const char *end)
{
	char buf[256];
	long n = 0;
	const char *tok;
	long len;

	while ((len = crash_token(&s, end, &tok)) > 0) {
		for (long i = 0; i < len && n < (long)sizeof(buf) - 2; i++) {
			buf[n++] = tok[i];
		}
		if (n < (long)sizeof(buf) - 2) {
			buf[n++] = ' ';
		}
	}
	if (n > 0) {
		n--;
	}
	buf[n] = 0;
	put("crash: /proc/interrupts: '");
	put(buf);
	put("'");
}

/*
 * What a row of /proc/interrupts is (show_interrupts, kernel/irq/proc.c;
 * the rows of the table in testdata/t6-proc-interrupts.txt):
 *  - The first line names the online CPUs, "CPU<n>" each: every other row
 *    has one count column per online CPU.
 *  - A row whose label is a number is an interrupt descriptor: the label,
 *    the counts, the chip's name, the hardware interrupt number (only when
 *    the interrupt has a domain), the trigger type (Level or Edge), then the
 *    names its handlers were requested with, comma separated.
 *  - Any other label (IPI0 ... IPI7, Err on arm64) is the architecture's
 *    own count with no chip (arch_show_interrupts).
 * A row with a handler is a numbered row with text after its trigger type.
 * Free_irq of the last handler shuts the interrupt down, so a row with
 * nothing there is left out of the check. A numbered row that does not read
 * that way (a chip name with a space, a missing column) is refused: it
 * cannot be told apart from one with a handler.
 *
 * 0: every row with a handler is on a measured chip; 1: refused, each row
 * that is not printed.
 */
static int crash_irq_table_check(const char *t, long n)
{
	const char *end = t + n, *line = t, *tok;
	long cpus = 0, len;
	int bad = 0, first = 1;

	while (line < end) {
		const char *eol = line, *p = line, *chip;
		long chiplen;

		while (eol < end && *eol != '\n') {
			eol++;
		}
		if (first) {
			first = 0;
			while ((len = crash_token(&p, eol, &tok)) > 0) {
				if (len > 3 && crash_starts(tok, len, "CPU") && crash_digits(tok + 3, len - 3)) {
					cpus++;
				}
			}
			if (cpus == 0) {
				put("crash: /proc/interrupts: no CPU header on its first line\n");
				return 1;
			}
			line = eol + 1;
			continue;
		}
		len = crash_token(&p, eol, &tok);
		if (len < 2 || tok[len - 1] != ':' || !crash_digits(tok, len - 1)) {
			line = eol + 1;   /* empty, or the architecture's row */
			continue;
		}
		for (long c = 0; c < cpus; c++) {
			len = crash_token(&p, eol, &tok);
			if (!crash_digits(tok, len)) {
				goto unread;
			}
		}
		chiplen = crash_token(&p, eol, &chip);
		if (chiplen == 0) {
			goto unread;
		}
		len = crash_token(&p, eol, &tok);
		if (crash_digits(tok, len)) {
			len = crash_token(&p, eol, &tok);
		}
		if (!(len == 5 && crash_starts(tok, len, "Level")) && !(len == 4 && crash_starts(tok, len, "Edge"))) {
			goto unread;
		}
		if (crash_token(&p, eol, &tok) > 0 && !crash_irq_chip_measured(chip, chiplen)) {
			char name[64];
			long k = 0;

			for (; k < chiplen && k < (long)sizeof(name) - 1; k++) {
				name[k] = chip[k];
			}
			name[k] = 0;
			crash_put_row(line, eol);
			put(": a handler on chip '");
			put(name);
			put("', not one the crash route was measured with\n");
			bad = 1;
		}
		line = eol + 1;
		continue;
unread:
		crash_put_row(line, eol);
		put(": not read as <number>: <one count per CPU> <chip> [<hwirq>] Level|Edge [<handlers>]\n");
		bad = 1;
		line = eol + 1;
	}
	return bad;
}

static char crash_irq_table[65536];

/*
 * /proc/interrupts read and checked, after crash_pwrkey_unbind.
 * 0: go on; 1: refuse (the caller's refusal re-binds the power key).
 */
static int crash_irq_check(void)
{
	long n = read_small("/proc/interrupts", crash_irq_table, sizeof(crash_irq_table));
	if (n < 0) {
		put("crash: /proc/interrupts cannot be read: error ");
		put_hex(-n);
		put("\n");
		return 1;
	}
	if (n >= (long)sizeof(crash_irq_table) - 1) {
		put("crash: /proc/interrupts fills the whole buffer: its end was not read\n");
		return 1;
	}
	if (crash_irq_table_check(crash_irq_table, n) != 0) {
		return 1;
	}
	put("crash: /proc/interrupts: every row with a handler is on a measured chip "
	    "(GICv3, GICv3-23, ITS-PCI-MSI-*, ITS-PCI-MSIX-*, rockchip_gpio_irq)\n");
	return 0;
}

#endif
