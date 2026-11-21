<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# linux_day_one

**Question.** What does the Verdin iMX95's own Linux show, before any image
of ours runs: the board and its kernel (M1), whether Linux drives the X4
display and the bridge's registers (M2, U20), what the System Manager lets
the AP set and whether the crash-kexec route can run (M3, U8), the golden
display registers (M4, U13, U16, U18), and the pre-study's B1 Linux half
(`/proc/iomem`, `/proc/interrupts`, panthor's lines and firmware,
`findmnt -T /var`)? `driver/docs/imx95-display.md` section 6,
`docs/notes/pre-study-imx95.md` section 6.

**What it does and does not do.** `day_one.sh` runs on the board (POSIX sh,
root) and writes the record to stdout. It reads files of `/proc`, `/sys`
(debugfs must already be mounted; it does not mount it), runs `dmesg`,
`uname`, `findmnt`, `fw_printenv ver`, `i2cdetect -r -y` on the I2C_2_DSI
bus (LPI2C3, found by its `of_node` `i2c@42530000`) and, only when the
bridge's driver exposes no regmap in debugfs, `i2cdump -y -r 0x00-0xff
<bus> 0x2c b` (no `-f`: a bound address is refused, which the record shows).
M4 reads registers through `/dev/mem` (python3's mmap, `O_RDONLY` and
`PROT_READ`, one child per word; or busybox `devmem ADDR 32`), only those
of the read list (the `#@read` lines of
`driver/docs/imx95-display-allow.txt`) and only behind their gates:
`display` needs the DSI connector connected and enabled (M2) and the DISPLAY
domain on in `pm_genpd_summary` (M3); `camera` also the CAMERA domain on. A
read of the DPU with DISPLAY off is U6, never tried. The read list leaves out
the DW host's GEN_PLD_DATA and INT_ST0/1 (they change on read). It writes no
register, no file and no I2C device, loads no module, mounts nothing and
touches no service.

## Run

From the host, the board named in the environment only:

```sh
VI95_SSH=<ssh destination> tools/probes/verdin_imx95/linux_day_one/run_day_one.sh > verdin-imx95-day-one.txt
```

`VI95_SSH_OPTS` adds ssh options, `VI95_SUDO` a prefix when the login is not
root (`sudo -n`). The script and the read list travel over the connection;
nothing is written to the board's storage. On the board itself:
`sh day_one.sh --readlist imx95-display-allow.txt`.

## Record format

Sections, each starting at its `[name]` line; `#` lines are comments or say
why something is absent (`# <file>: not readable`, `# <tool>: not
installed`, `# <block>: gate display does not hold, not read`):

    [m1]  model, compatible, uname, kernel release, os-release, overlays, the
          U-Boot version (device tree and env), dpu95's dmesg count
    [b1]  iomem:, interrupts:, gpu dmesg:, gpu firmware:, findmnt /var:,
          meminfo:, cmdline:, nvmem:
    [m2]  <connector> status|enabled|dpms|mode VALUE; dsi connector connected
          and enabled: 0|1; display dmesg:; i2c_2_dsi bus:; i2cdetect:;
          bridge regmap: (or bridge i2cdump:)
    [m3]  sm dmesg:, scmi info <n>/info/<file>: (the SM's vendor and
          implementation version), clk: rows, genpd:, display domain on:,
          camera domain on:, config: (KEXEC, CRASH_DUMP, STRICT_DEVMEM, ...),
          crash reservation:
    [m4]  reader:, then 0xADDRESS 0xVALUE NAME+0xOFFSET (VALUE is ERR when
          the read failed)

## Reading it

- M1: `model` should name "Verdin iMX95 ... Mallow"; `dpu95 in this kernel:
  0` means a kernel without NXP's display driver (mainline): route (a) of
  the note is out.
- M2: `dsi connector connected and enabled: 1` and a `1280x800` mode: Linux
  drives X4. `bridge regmap: 18: ..` is `LVDS_FMT` (U20: 0x18 or 0x78).
  `i2cdetect` should show 0x2c (UU when bound) and 0x41 (the touch, U10).
- M3: `sm dmesg` and `scmi info` carry the SM's version (pre-study U1);
  `config: CONFIG_KEXEC=y`, `CONFIG_CRASH_DUMP=y` and a `crash
  reservation` line are U8; `CONFIG_STRICT_DEVMEM=y` makes M4's reads `ERR`
  (report it, do not change the kernel).
- M4: the golden dump of the running picture; compare with our image's
  read-backs through the allow file's bits. `combo_phy_csr+0x4` is U13,
  `+0x0` and `+0x8` U16, `dw_dsi+0x0` U18.

## Selftest

`day_one_selftest.sh` runs the script on a fake board (a fake `/proc`, `/sys`
and `/etc`, fake `dmesg`, `i2cdetect`, `i2cdump`, `uname`, `findmnt`, a
register fixture) and checks the record's lines, that M4 reads only behind
its gates (CAMERA off: no DSI word; DISPLAY off: nothing), that the allow
file and its read list parse and leave out the read-sensitive words, and
that the script holds no write-capable construct.
