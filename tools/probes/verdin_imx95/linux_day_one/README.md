<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# linux_day_one

**Question.** What does the Verdin iMX95's own Linux show, before any image
of ours runs: the board and its kernel (M1), which display Linux drives (the
X12 LVDS panel of the owner's board, or a DSI display on X4 with its
bridge's registers, U20) and its backlight (M2), what the System Manager
lets the AP set and whether the crash-kexec route can run (M3, U8), the
golden display registers (M4; U23 on X12, U13, U16, U18 on X4), and the
pre-study's B1 Linux half
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
M4 reads registers through `/dev/mem`, only those of the read list (the
`#@read` lines of `driver/docs/imx95-display-allow.txt`), never a
`#@noread` word, and only behind their gates: `display` needs an LVDS or DSI
connector connected and enabled (M2) and the DISPLAY domain on in
`pm_genpd_summary` (M3); `lvds` an LVDS connector and DISPLAY; `dsi` a DSI
connector and DISPLAY; `camera` a DSI connector, DISPLAY and the CAMERA
domain. A read of the DPU with DISPLAY off is U6, never tried. The reader is
chosen by trying: python3's mmap (`O_RDONLY`, `PROT_READ`, one child per
word), then `devmem2 ADDR w`, then busybox `devmem ADDR 32`; the first that
reads the first gated word of the list is used, and the record says which
were tried and why they were passed over. `devmem2` opens `/dev/mem`
read-write but writes nothing without a third argument, which is never
given. `#@noread` holds the words a read of which changes state (the DW
host's GEN_PLD_DATA and INT_ST0/1) and the seven DPU words whose read
faulted on the board (write-only registers and reserved words). It writes
no register, no file (but the wrapper's two copies, below) and no I2C
device, loads no module, mounts nothing and touches no service.

What the board showed on 2026-11-25 (read by the owner's session by hand,
before these fixes) is `driver/docs/imx95-display.md` section 6 and the dump
`driver/docs/imx95-display-dump-linux.txt`. Three defects of this tooling
showed there and are fixed: (1) the wrapper passed the whole script as one
ssh command-line argument and the board's Dropbear (v2022.83) reset the
connection; (2) the reader was python3, whose `mmap` module the board's
image lacks ("ModuleNotFoundError: No module named 'mmap'"), and the image
has no busybox `devmem`, only `/usr/bin/devmem2`; (3) the gate asked for a
DSI connector, which the LVDS panel never gives, so nothing was read, and
seven words of the old list fault on read.

## Run

From the host, the board named in the environment only:

```sh
VI95_SSH=<ssh destination> tools/probes/verdin_imx95/linux_day_one/run_day_one.sh > verdin-imx95-day-one.txt
```

`VI95_SSH_OPTS` adds ssh options, `VI95_SUDO` a prefix when the login is not
root (`sudo -n`). Every ssh command line is short: the wrapper makes a
fresh directory under the board's `/tmp` (a tmpfs on Toradex's image;
`VI95_REMOTE_TMP` names another absolute path), copies `day_one.sh` and the
read list into it over the connection's stdin (`cat >`), runs the script
from there and removes the directory at the end, also after an error.
Nothing reaches the board's storage. An argument with a character the
remote shell would read is refused before any ssh. On the board itself:
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
          and enabled: 0|1; lvds connector connected and enabled: 0|1;
          display dmesg:; backlight <name>:; pwm:; i2c_2_dsi bus:;
          i2cdetect:; bridge regmap: (or bridge i2cdump:)
    [m3]  sm dmesg:, scmi info <n>/info/<file>: (the SM's vendor and
          implementation version), clk: rows, genpd:, display domain on:,
          camera domain on:, config: (KEXEC, CRASH_DUMP, STRICT_DEVMEM, ...),
          crash reservation:
    [m4]  # reader <kind>: ... (each one tried), reader:, then 0xADDRESS
          0xVALUE NAME+0xOFFSET (VALUE is ERR when the read failed)

## Reading it

- M1: `model` names the module; Toradex's device tree says "on Verdin
  Development Board" whatever the carrier (`fdt_board=dev` by default; the
  owner's carrier is a Mallow). `dpu95 in this kernel: 0` means no dmesg
  line names `dpu95`; the M3 `config:` line (`CONFIG_DRM_IMX95_DPU`) says
  whether the kernel has the driver.
- M2: `lvds connector connected and enabled: 1` and a `1280x800` mode: Linux
  drives the X12 panel (the owner's board); `pwm:` and `backlight` give the
  backlight as Linux runs it. `dsi connector connected and enabled: 1`:
  Linux drives X4; then `bridge regmap: 18: ..` is `LVDS_FMT` (U20: 0x18 or
  0x78) and `i2cdetect` should show 0x2c and 0x41.
- M3: `sm dmesg` and `scmi info` carry the SM's version (pre-study U1);
  `config: CONFIG_KEXEC=y`, `CONFIG_CRASH_DUMP=y` and a `crash
  reservation` line are U8; `CONFIG_STRICT_DEVMEM=y` makes M4's reads `ERR`
  (report it, do not change the kernel).
- M4: the golden dump of the running picture; compare with our image's
  read-backs through the allow file's bits (the X12 path's comparison is a
  host test, `soc::imx95::lvds_tests`). `dpu_Dither0_POLARITYCTRL0+0x0` is
  U23. On a DSI display: `combo_phy_csr+0x4` is U13, `+0x0` and `+0x8` U16,
  `dw_dsi+0x0` U18.

## Selftest

`day_one_selftest.sh` runs the script on a fake board (a fake `/proc`, `/sys`
and `/etc`, fake `dmesg`, `i2cdetect`, `i2cdump`, `uname`, `findmnt`, a
python3 without `mmap` and a `devmem2` as the board's, a register fixture)
and checks the record's lines, that the reader is chosen only once it has
read (python3 passed over, devmem2 taken; none: M4 says so and reads
nothing), that M4 reads only behind its gates (the LVDS panel: the LVDS CSR
and the DPU, no DSI word; a DSI panel: no LVDS CSR; CAMERA off: no DSI word;
DISPLAY off: nothing, no reader tried), that the allow file and its read
list parse and cover none of the `#@noread` words, that the script holds no
write-capable construct and calls devmem2 only as `devmem2 ADDR w`; and it
runs `run_day_one.sh` through a fake ssh: five short commands (no script in
an argument), the run from the board's `/tmp`, the copy removed, a
shell-active argument refused before any ssh.
