// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * crash_irq.h on the host: kexec_zephyr's steps before the crash route's
 * panic (unbind the PMIC power key, read /proc/interrupts, re-bind on a
 * refusal) against a made-up /sys and /proc. crash_irq_selftest.sh builds
 * and runs it:
 *
 *   crash_irq_test <testdata/t6-proc-interrupts.txt>
 *
 * The fixture is the NanoPC-T6's /proc/interrupts with the power-key driver
 * bound (Linux 6.18.55, 2026-11-15), cut down to the header, some rows of
 * each chip it had and the architecture's rows. The table "after the
 * unbind" is the fixture without the two rows of chip rk806: on the board
 * the unbind took exactly those away.
 *
 * The fakes stand in for kexec_zephyr.c's raw system calls: list_dir lists
 * the driver directory, write_file records every write (and fails the ones
 * a case asks it to), read_small hands out the table, the one after the
 * unbind once an unbind succeeded.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char out[1 << 16];
static size_t outn;

static void put(const char *s)
{
	size_t n = strlen(s);

	if (outn + n < sizeof(out)) {
		memcpy(out + outn, s, n + 1);
		outn += n;
	}
}

static void put_hex(unsigned long v)
{
	char buf[24];

	snprintf(buf, sizeof(buf), "0x%lx", v);
	put(buf);
}

static struct {
	const char *dir;        /* NUL-separated names; NULL: no directory */
	size_t dirlen;
	const char *table;      /* /proc/interrupts */
	const char *table_after; /* /proc/interrupts after an unbind; NULL: the same */
	int fail_unbind, fail_bind, unbound;
	char writes[4096];      /* "path=text\n" for every write */
} fk;

static long read_small(const char *path, char *buf, long max)
{
	const char *t = fk.unbound && fk.table_after ? fk.table_after : fk.table;
	long n;

	if (strcmp(path, "/proc/interrupts") != 0) {
		return -2;
	}
	n = (long)strlen(t);
	if (n > max - 1) {
		n = max - 1;
	}
	memcpy(buf, t, (size_t)n);
	buf[n] = 0;
	return n;
}

static int ends_with(const char *s, const char *tail)
{
	size_t a = strlen(s), b = strlen(tail);

	return a >= b && strcmp(s + a - b, tail) == 0;
}

static int write_file(const char *path, const char *text)
{
	size_t n = strlen(fk.writes);

	snprintf(fk.writes + n, sizeof(fk.writes) - n, "%s=%s\n", path, text);
	if (ends_with(path, "/rk805-pwrkey/unbind")) {
		if (fk.fail_unbind) {
			return -19;
		}
		fk.unbound = 1;
		return 0;
	}
	if (ends_with(path, "/rk805-pwrkey/bind")) {
		return fk.fail_bind ? -19 : 0;
	}
	return -2;
}

static long list_dir(const char *dir, char *names, long max)
{
	if (strcmp(dir, "/sys/bus/platform/drivers/rk805-pwrkey") != 0 || !fk.dir) {
		return -2;
	}
	if ((long)fk.dirlen > max) {
		return -7;
	}
	memcpy(names, fk.dir, fk.dirlen);
	return (long)fk.dirlen;
}

#include "crash_irq.h"

static int run, failed;

static void check(const char *what, int ok)
{
	run++;
	printf("%s %s\n", what, ok ? "ok" : "FAIL");
	if (!ok) {
		failed++;
		printf("---- what the steps printed:\n%s---- writes:\n%s----\n", out, fk.writes);
	}
}

/* kexec_zephyr's two steps, with what it does between them left out. */
static int crash_irq_gate(void)
{
	crash_pwrkey_unbind();
	return crash_irq_check();
}

static int has(const char *s, const char *w)
{
	return strstr(s, w) != NULL;
}

/* A new board: nothing printed, written or unbound yet (a re-bind empties
 * the list of what an earlier case unbound). */
static void board(const char *dir, size_t dirlen, const char *table, const char *table_after)
{
	(void)crash_pwrkey_rebind();
	memset(&fk, 0, sizeof(fk));
	fk.dir = dir;
	fk.dirlen = dirlen;
	fk.table = table;
	fk.table_after = table_after;
	out[0] = 0;
	outn = 0;
}

/* The measured driver directory (the device name changes per boot). */
static const char measured_dir[] = ".\0..\0bind\0module\0rk805-pwrkey.4.auto\0uevent\0unbind\0";
static const char attrs_only[] = ".\0..\0bind\0module\0uevent\0unbind\0";
#define UNBIND "/sys/bus/platform/drivers/rk805-pwrkey/unbind=rk805-pwrkey.4.auto\n"
#define BIND "/sys/bus/platform/drivers/rk805-pwrkey/bind=rk805-pwrkey.4.auto\n"
#define FALL_ROW "'86: 0 0 0 0 0 0 0 0 rk806 0 Edge rk805_pwrkey_fall'"
#define RISE_ROW "'87: 0 0 0 0 0 0 0 0 rk806 1 Edge rk805_pwrkey_rise'"

/* The text with every line that holds w left out. */
static char *without(const char *text, const char *w)
{
	char *r = calloc(1, strlen(text) + 1);
	const char *p = text;

	while (*p) {
		const char *e = strchr(p, '\n');
		size_t n = e ? (size_t)(e - p) + 1 : strlen(p);
		char line[512];

		snprintf(line, sizeof(line), "%.*s", (int)n, p);
		if (!strstr(line, w)) {
			strncat(r, p, n);
		}
		p += n;
	}
	return r;
}

/* The text with row put in before the line that starts with "IPI0:". */
static char *with_row(const char *text, const char *row)
{
	const char *at = strstr(text, "\nIPI0:");
	char *r = calloc(1, strlen(text) + strlen(row) + 1);

	if (!at) {
		return r;
	}
	strncat(r, text, (size_t)(at - text) + 1);
	strcat(r, row);
	strcat(r, at + 1);
	return r;
}

static char *read_fixture(const char *path)
{
	FILE *f = fopen(path, "r");
	char *t;
	long n;

	if (!f) {
		return NULL;
	}
	fseek(f, 0, SEEK_END);
	n = ftell(f);
	fseek(f, 0, SEEK_SET);
	t = calloc(1, (size_t)n + 1);
	if (fread(t, 1, (size_t)n, f) != (size_t)n) {
		fclose(f);
		free(t);
		return NULL;
	}
	fclose(f);
	return t;
}

int main(int argc, char **argv)
{
	char *bound, *after, *unknown, *big;
	int rc;

	if (argc != 2 || !(bound = read_fixture(argv[1]))) {
		fprintf(stderr, "usage: crash_irq_test <testdata/t6-proc-interrupts.txt>\n");
		return 2;
	}
	after = without(bound, " rk806 ");
	unknown = with_row(after, "200:          0          0          0          0          0          0          0          0 unmeasured   3 Edge      button\n");
	check("the fixture has the two rows of chip rk806 and the table after the unbind has none",
	      has(bound, "rk805_pwrkey_fall") && has(bound, "rk805_pwrkey_rise") &&
	      !has(after, "rk805_pwrkey") && has(after, "rockchip_gpio_irq   7 Level     rk806"));

	/* The measured case: the device is unbound, its two rows go. */
	board(measured_dir, sizeof(measured_dir) - 1, bound, after);
	rc = crash_irq_gate();
	check("bound power key: it is unbound by the name listed, and nothing else is written", strcmp(fk.writes, UNBIND) == 0);
	check("bound power key: once unbound, every row left is on a measured chip and the steps go on", rc == 0);
	check("... the line says the device was unbound", has(out, "crash: rk805-pwrkey.4.auto unbound from rk805-pwrkey\n"));
	check("... the PMIC's parent row on rockchip_gpio_irq stays and is let through", !has(out, "'85:"));
	check("... and re-binding after that puts back exactly what was unbound", crash_pwrkey_rebind() == 0 && has(fk.writes, UNBIND BIND));

	/* Not unbound: the unbind fails, the rows stay. */
	board(measured_dir, sizeof(measured_dir) - 1, bound, after);
	fk.fail_unbind = 1;
	rc = crash_irq_gate();
	check("unbind fails: refused", rc == 1);
	check("... the failed unbind is reported with its error", has(out, "crash: rk805-pwrkey.4.auto: unbind from rk805-pwrkey FAILED: error 0x13\n"));
	check("... both rk806 rows are named, with the chip",
	      has(out, FALL_ROW) && has(out, RISE_ROW) && has(out, "a handler on chip 'rk806'"));
	fk.fail_unbind = 0;
	check("... and there is nothing to re-bind", crash_pwrkey_rebind() == 0 && strcmp(fk.writes, UNBIND) == 0);

	/* Not unbound: no driver directory, but the rows are there. */
	board(NULL, 0, bound, NULL);
	rc = crash_irq_gate();
	check("no power-key driver but rk806 rows in the table: refused naming them", rc == 1 && has(out, FALL_ROW) && has(out, RISE_ROW));
	check("... nothing is written", fk.writes[0] == 0);

	/* No power-key device at all: not an error. */
	board(NULL, 0, after, NULL);
	rc = crash_irq_gate();
	check("no rk805-pwrkey directory and no rk806 rows: the steps go on", rc == 0 && fk.writes[0] == 0);
	check("... and the line says there was no power key to unbind", has(out, "crash: no /sys/bus/platform/drivers/rk805-pwrkey: no power key to unbind\n"));
	board(attrs_only, sizeof(attrs_only) - 1, after, NULL);
	rc = crash_irq_gate();
	check("a driver directory with no device in it: the steps go on, no write", rc == 0 && fk.writes[0] == 0);
	check("... and the line says no device is bound", has(out, "crash: no device bound to rk805-pwrkey\n"));

	/* An unknown chip: refused, and the refusal re-binds. */
	board(measured_dir, sizeof(measured_dir) - 1, unknown, unknown);
	rc = crash_irq_gate();
	check("a row with a handler on a chip not in the list: refused",
	      rc == 1 && has(out, "'200: 0 0 0 0 0 0 0 0 unmeasured 3 Edge button': a handler on chip 'unmeasured'"));
	check("... only that row is named", !has(out, "'19:") && !has(out, "'74:") && !has(out, "'130:") && !has(out, "'163:"));
	rc = crash_pwrkey_rebind();
	check("... the refusal re-binds the device it unbound", rc == 0 && strcmp(fk.writes, UNBIND BIND) == 0);
	check("... and says so", has(out, "crash: rk805-pwrkey.4.auto bound to rk805-pwrkey again\n"));
	check("... a second re-bind writes nothing more", crash_pwrkey_rebind() == 0 && strcmp(fk.writes, UNBIND BIND) == 0);

	board(measured_dir, sizeof(measured_dir) - 1, unknown, unknown);
	(void)crash_irq_gate();
	fk.fail_bind = 1;
	rc = crash_pwrkey_rebind();
	check("a re-bind that fails is counted and reported, not hidden",
	      rc == 1 && has(out, "crash: rk805-pwrkey.4.auto: bind to rk805-pwrkey FAILED: error 0x13; the power key stays unbound\n"));

	/* The table as kexec_zephyr reads it: CPUs 1-7 already offline. */
	board(NULL, 0, "           CPU0\n"
		       " 19:      24730    GICv3  26 Level     arch_timer\n"
		       " 74:          0 ITS-PCI-MSI-0003:30:00.0   8 Edge      PCIe PME, PCIe bwctrl\n"
		       " 86:          0    rk806   0 Edge      rk805_pwrkey_fall\n"
		       "IPI0:      6472       Rescheduling interrupts\n"
		       "Err:          0\n", NULL);
	rc = crash_irq_gate();
	check("one count column (only CPU 0 online): the rk806 row is still found and named",
	      rc == 1 && has(out, "'86: 0 rk806 0 Edge rk805_pwrkey_fall'") && !has(out, "'19:") && !has(out, "'74:"));

	/* The chip names are matched exactly, or by the two prefixes. */
	board(NULL, 0, "           CPU0\n"
		       " 62:          0 GICv3-24   0 Level     arm-pmu\n"
		       " 63:          0 GICv3x   1 Level     arm-pmu\n"
		       " 64:          0 ITS-PCI-MSI   2 Edge      aerdrv\n"
		       " 65:          0 ITS-PCI-MSIX-0002:21:00.0   0 Edge      lan2\n"
		       " 66:          0 rockchip_gpio_irq   20 Edge      x\n", NULL);
	rc = crash_irq_gate();
	check("a chip name close to a measured one is refused (GICv3-24, GICv3x, ITS-PCI-MSI without its device)",
	      rc == 1 && has(out, "'62:") && has(out, "'63:") && has(out, "'64:") && !has(out, "'65:") && !has(out, "'66:"));

	/* What a row with a handler is. */
	board(NULL, 0, "           CPU0       CPU1\n"
		       " 90:          5          0 unmeasured   3 Level\n", NULL);
	rc = crash_irq_gate();
	check("a row with nothing after its trigger type has no handler: let through on any chip", rc == 0);
	board(NULL, 0, "           CPU0       CPU1\n"
		       " 91:          0          0 two words   3 Level     x\n", NULL);
	rc = crash_irq_gate();
	check("a row that does not read as counts, chip, [hwirq], Level|Edge is refused, named",
	      rc == 1 && has(out, "'91: 0 0 two words 3 Level x': not read as"));
	board(NULL, 0, "           CPU0       CPU1\n"
		       " 92:          0 GICv3  26 Level     x\n", NULL);
	rc = crash_irq_gate();
	check("a row with fewer count columns than the header has CPUs is refused", rc == 1 && has(out, "'92: 0 GICv3 26 Level x': not read as"));
	board(NULL, 0, " 19:      24730    GICv3  26 Level     arch_timer\n", NULL);
	rc = crash_irq_gate();
	check("a table without its CPU header is refused", rc == 1 && has(out, "no CPU header"));
	big = calloc(1, sizeof(crash_irq_table) + 64);
	memset(big, ' ', sizeof(crash_irq_table) + 63);
	memcpy(big, "           CPU0\n", 16);
	board(NULL, 0, big, NULL);
	rc = crash_irq_gate();
	check("a table that fills the whole buffer is refused (its end was not read)", rc == 1 && has(out, "fills the"));

	printf("crash_irq_selftest: %d checks run, %d failed\n", run, failed);
	free(bound);
	free(after);
	free(unknown);
	free(big);
	return failed == 0 && run > 0 ? 0 : 1;
}
