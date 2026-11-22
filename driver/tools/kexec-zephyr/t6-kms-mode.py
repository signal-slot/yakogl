#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Hold the adopt's mode on the NanoPC-T6's HDMI under Linux, for a crash jump.

    t6-kms-mode.py hold [--card PATH] [--connector NAME] [--max-hold S] [--log FILE]
        On the board, as root, with no display server. Opens the DRM card
        (default /dev/dri/card0) as its master, finds the connector (default
        HDMI-A-1) and the CRTC that drives it, and picks the connector's mode
        whose timing is MODE exactly: the clock, the eight timing numbers and
        both sync flags (mode_matches()). It sets that mode with a dumb
        buffer of its own (legacy SETCRTC), reads the CRTC back, prints one line
        and leaves a child process that holds the mode: the parent exits and
        the child keeps the card open as its master. The child gives the
        CRTC back as it found it (its frame buffer, position and mode) on
        SIGTERM, SIGINT or SIGHUP, or by itself after --max-hold seconds
        (default 600), then closes the card. A crash jump ends Linux with the
        mode still set: the crash route shuts no device down.
    t6-kms-mode.py --self-test
        The packing, the ioctl numbers, the mode rule and the whole hold and
        give-back order against a made-up DRM device. On the host, no board.

Why it exists. The Zephyr image adopts the picture Linux leaves on the
crash route and accepts one mode only, MODE below (the Rust
board::nanopc_t6_display::MODE; the self-test compares the two). Linux
picks the sink's preferred mode, so with a sink whose EDID prefers another
mode (an HDMI capture unit: 1920x1080) the adopt would stop at its mode
check. nanopc-t6.sh --crash runs this before the jump and reads VP0's mode
back from Linux itself (driver/docs/nanopc-t6-display.md, section 2(a)).

What it leaves behind: nothing that outlives the process. It writes no file
(except --log, which the runner puts in /tmp/yakogl-kexec), loads no module
and sets no property. When the process ends, however it ends (SIGKILL too),
the kernel removes its frame buffer and its master, and the last close of
the card lets fbdev set its own mode again (linux v6.18
drivers/gpu/drm/drm_file.c:253, :263-264, :406-411, :440-441;
drm_client_event.c:104-123). The give-back on a signal does the same first,
explicitly, with the CRTC's own values.

Exit: 0 the mode is set and held (the line names the holder's pid). 3
refused, nothing changed: no such card or connector, the connector is not
connected or drives no CRTC, another master holds the card, or the
connector lists no mode with MODE's timing (the line lists the modes of the
same size). 1 the set or its read-back failed; the CRTC was given back
before the exit (the line says whether that worked). 2 usage.

Only the Python standard library: the board has python3 and no DRM tools.
"""

import array
import errno
import fcntl
import os
import signal
import struct
import sys
from collections import namedtuple

# The mode the Zephyr image adopts (board::nanopc_t6_display::MODE): 1024x768
# at 60 Hz, 65 MHz, VESA DMT. Clock in kHz as DRM gives it.
MODE = {
    "clock": 65000,
    "hdisplay": 1024, "hsync_start": 1048, "hsync_end": 1184, "htotal": 1344,
    "vdisplay": 768, "vsync_start": 771, "vsync_end": 777, "vtotal": 806,
}
# include/uapi/drm/drm_mode.h: DRM_MODE_FLAG_*
FLAG_PHSYNC, FLAG_NHSYNC, FLAG_PVSYNC, FLAG_NVSYNC = 0x1, 0x2, 0x4, 0x8
FLAG_INTERLACE, FLAG_DBLSCAN = 0x10, 0x20
# The flags that change the signal: both polarities of both syncs, interlace
# and double scan. MODE has negative syncs, progressive.
FLAG_TIMING = FLAG_PHSYNC | FLAG_NHSYNC | FLAG_PVSYNC | FLAG_NVSYNC | FLAG_INTERLACE | FLAG_DBLSCAN
MODE_FLAGS = FLAG_NHSYNC | FLAG_NVSYNC
CONNECTED = 1  # drm_mode_get_connector.connection: DRM_MODE_CONNECTED
CONNECTOR_TYPES = {10: "DisplayPort", 11: "HDMI-A", 12: "HDMI-B", 14: "eDP", 16: "DSI"}

# The uapi structs, little endian, no padding (each is a multiple of its
# alignment already; the self-test pins the sizes to the kernel header's).
MODEINFO = struct.Struct("<IHHHHHHHHHHIII32s")   # struct drm_mode_modeinfo, 68
CARD_RES = struct.Struct("<QQQQIIIIIIII")        # struct drm_mode_card_res, 64
GET_CONNECTOR = struct.Struct("<QQQQIIIIIIIIIIII")  # struct drm_mode_get_connector, 80
GET_ENCODER = struct.Struct("<IIIII")            # struct drm_mode_get_encoder, 20
CRTC = struct.Struct("<QIIIIIII68s")             # struct drm_mode_crtc, 104
CREATE_DUMB = struct.Struct("<IIIIIIQ")          # struct drm_mode_create_dumb, 32
FB_CMD = struct.Struct("<IIIIIII")               # struct drm_mode_fb_cmd, 28
U32 = struct.Struct("<I")                        # RMFB's id, drm_mode_destroy_dumb


def _ioc(direction, nr, size):
    """include/uapi/asm-generic/ioctl.h, which arm64 uses: dir 2 bits, size
    14 bits, type 8 bits ('d' for DRM), nr 8 bits."""
    return direction << 30 | size << 16 | ord("d") << 8 | nr


def _iowr(nr, st):
    return _ioc(3, nr, st.size)


IOCTL_SET_MASTER = _ioc(0, 0x1E, 0)
IOCTL_GETRESOURCES = _iowr(0xA0, CARD_RES)
IOCTL_GETCRTC = _iowr(0xA1, CRTC)
IOCTL_SETCRTC = _iowr(0xA2, CRTC)
IOCTL_GETENCODER = _iowr(0xA6, GET_ENCODER)
IOCTL_GETCONNECTOR = _iowr(0xA7, GET_CONNECTOR)
IOCTL_ADDFB = _iowr(0xAE, FB_CMD)
IOCTL_RMFB = _iowr(0xAF, U32)
IOCTL_CREATE_DUMB = _iowr(0xB2, CREATE_DUMB)
IOCTL_DESTROY_DUMB = _iowr(0xB4, U32)

Mode = namedtuple("Mode", "clock hdisplay hsync_start hsync_end htotal hskew vdisplay vsync_start "
                          "vsync_end vtotal vscan vrefresh flags type name")


def mode_unpack(raw):
    f = list(MODEINFO.unpack(raw))
    f[-1] = f[-1].split(b"\0", 1)[0].decode("ascii", "replace")
    return Mode(*f)


def mode_text(m):
    return '"%s" %d kHz h %d %d %d %d v %d %d %d %d %shsync %svsync%s' % (
        m.name, m.clock, m.hdisplay, m.hsync_start, m.hsync_end, m.htotal,
        m.vdisplay, m.vsync_start, m.vsync_end, m.vtotal,
        "-" if m.flags & FLAG_NHSYNC else "+" if m.flags & FLAG_PHSYNC else "?",
        "-" if m.flags & FLAG_NVSYNC else "+" if m.flags & FLAG_PVSYNC else "?",
        " interlace" if m.flags & FLAG_INTERLACE else "")


def mode_matches(m):
    """The rule: MODE's clock and eight timing numbers, negative syncs, not
    interlaced, not double scan, no skew, no repeat of lines. The refresh,
    the type bits and the name do not matter: they do not change the signal."""
    return (all(getattr(m, k) == v for k, v in MODE.items())
            and m.flags & FLAG_TIMING == MODE_FLAGS
            and m.hskew == 0 and m.vscan in (0, 1))


def mode_pick(modes):
    """The first mode of the list that matches, or None. modes: raw bytes."""
    for raw in modes:
        if mode_matches(mode_unpack(raw)):
            return raw
    return None


class Refused(Exception):
    """Nothing was changed."""


class SetFailed(Exception):
    """The set or its read-back failed; the CRTC was given back (the text
    says how that went)."""


class Drm:
    """The legacy KMS ioctls this tool needs, on an open card. ioctl is
    fcntl.ioctl on the board and a made-up device in the self-test."""

    def __init__(self, fd, ioctl=fcntl.ioctl):
        self.fd = fd
        self.ioctl = ioctl

    def call(self, req, st, *fields):
        buf = bytearray(st.pack(*fields))
        self.ioctl(self.fd, req, buf, True)
        return st.unpack(bytes(buf))

    def set_master(self):
        self.ioctl(self.fd, IOCTL_SET_MASTER, 0)

    def resources(self):
        """(crtc ids, connector ids). Asked twice: counts, then the arrays;
        again if a count grew in between."""
        n = CARD_RES.unpack(bytes(CARD_RES.size))
        while True:
            crtcs = array.array("I", [0] * n[5])
            conns = array.array("I", [0] * n[6])
            got = self.call(IOCTL_GETRESOURCES, CARD_RES, 0, crtcs.buffer_info()[0] if n[5] else 0,
                            conns.buffer_info()[0] if n[6] else 0, 0, 0, n[5], n[6], 0, 0, 0, 0, 0)
            if got[5] <= n[5] and got[6] <= n[6]:
                return list(crtcs[:got[5]]), list(conns[:got[6]])
            n = got

    def connector(self, cid):
        """(type, type id, connection, encoder id, modes as raw bytes). The
        first call has count_modes 0, so the kernel probes the connector
        again when this file is the master (drm_connector.c:3357-3361)."""
        nmodes = 0
        while True:
            modes = array.array("B", bytes(MODEINFO.size * nmodes))
            got = self.call(IOCTL_GETCONNECTOR, GET_CONNECTOR, 0, modes.buffer_info()[0] if nmodes else 0,
                            0, 0, nmodes, 0, 0, 0, cid, 0, 0, 0, 0, 0, 0, 0)
            # the kernel copies the modes only when they all fit (:3388)
            if nmodes and got[4] <= nmodes:
                raw = modes.tobytes()
                return (got[9], got[10], got[11], got[7],
                        [raw[i * MODEINFO.size:(i + 1) * MODEINFO.size] for i in range(got[4])])
            if got[4] == 0:
                return got[9], got[10], got[11], got[7], []
            nmodes = got[4]

    def encoder_crtc(self, eid):
        return self.call(IOCTL_GETENCODER, GET_ENCODER, eid, 0, 0, 0, 0)[2]

    def get_crtc(self, crtc):
        """(fb id, x, y, mode_valid, raw mode)."""
        got = self.call(IOCTL_GETCRTC, CRTC, 0, 0, crtc, 0, 0, 0, 0, 0, bytes(MODEINFO.size))
        return got[3], got[4], got[5], got[7], got[8]

    def set_crtc(self, crtc, fb, x, y, conns, raw_mode):
        ids = array.array("I", conns)
        self.call(IOCTL_SETCRTC, CRTC, ids.buffer_info()[0] if conns else 0, len(conns), crtc, fb, x, y, 0,
                  1 if raw_mode is not None else 0, raw_mode if raw_mode is not None else bytes(MODEINFO.size))

    def create_dumb(self, w, h):
        """(handle, pitch). What the buffer shows does not matter: the image
        puts its own window on the screen."""
        got = self.call(IOCTL_CREATE_DUMB, CREATE_DUMB, h, w, 32, 0, 0, 0, 0)
        return got[4], got[5]

    def add_fb(self, w, h, pitch, handle):
        return self.call(IOCTL_ADDFB, FB_CMD, 0, w, h, pitch, 32, 24, handle)[0]

    def rm_fb(self, fb):
        self.call(IOCTL_RMFB, U32, fb)

    def destroy_dumb(self, handle):
        self.call(IOCTL_DESTROY_DUMB, U32, handle)


def connector_name(ctype, type_id):
    return "%s-%d" % (CONNECTOR_TYPES.get(ctype, "type%d" % ctype), type_id)


def take(drm, name):
    """Set MODE on the connector called name. Returns what give_back needs.
    Raises Refused before any change (a read the card refuses included); on
    a failure after a change it gives the CRTC back first and raises
    SetFailed."""
    def ask(what, fn, *args):
        try:
            return fn(*args)
        except OSError as e:
            raise Refused("cannot %s (%s)" % (what, os.strerror(e.errno) if e.errno else e))

    try:
        drm.set_master()
    except OSError as e:
        raise Refused("cannot be the card's DRM master (%s): another master holds it (a display server?)"
                      % os.strerror(e.errno))
    _, conns = ask("read the card's connectors", drm.resources)
    found = None
    for cid in conns:
        c = ask("read connector %d" % cid, drm.connector, cid)
        if connector_name(c[0], c[1]) == name:
            found = (cid,) + c
            break
    if found is None:
        raise Refused("no connector %s on the card" % name)
    cid, _, _, connection, eid, modes = found
    if connection != CONNECTED:
        raise Refused("%s is not connected (connection %d)" % (name, connection))
    crtc = ask("read encoder %d of %s" % (eid, name), drm.encoder_crtc, eid) if eid else 0
    if not crtc:
        raise Refused("%s drives no CRTC (encoder %d): Linux shows nothing on it to adopt" % (name, eid))
    raw = mode_pick(modes)
    if raw is None:
        same = [mode_text(mode_unpack(r)) for r in modes
                if mode_unpack(r).hdisplay == MODE["hdisplay"] and mode_unpack(r).vdisplay == MODE["vdisplay"]]
        raise Refused("%s lists no mode with the adopt's timing (%d modes; of size %dx%d: %s)"
                      % (name, len(modes), MODE["hdisplay"], MODE["vdisplay"], "; ".join(same) or "none"))
    old = ask("read CRTC %d" % crtc, drm.get_crtc, crtc)
    w, h = MODE["hdisplay"], MODE["vdisplay"]
    handle, pitch = ask("get a %dx%d buffer from the card" % (w, h), drm.create_dumb, w, h)
    state = {"crtc": crtc, "conn": cid, "old": old, "handle": handle, "fb": 0, "mode": raw}
    try:
        state["fb"] = drm.add_fb(w, h, pitch, handle)
        drm.set_crtc(crtc, state["fb"], 0, 0, [cid], raw)
        now = drm.get_crtc(crtc)
        if now[0] != state["fb"] or not now[3] or not mode_matches(mode_unpack(now[4])):
            raise RuntimeError("CRTC %d reads back fb %d, mode %s after the set"
                               % (crtc, now[0], mode_text(mode_unpack(now[4])) if now[3] else "none"))
    except (OSError, RuntimeError) as e:
        why = os.strerror(e.errno) if isinstance(e, OSError) and e.errno else str(e)
        raise SetFailed("%s; given back: %s" % (why, give_back(drm, state)))
    return state


def give_back(drm, state):
    """The CRTC as take() found it, then our buffer away. Returns a text that
    says what happened; never raises (the kernel's own release after the
    close is the fallback)."""
    fb_old, x, y, valid, raw_old = state["old"]
    what = []
    try:
        if valid and fb_old:
            drm.set_crtc(state["crtc"], fb_old, x, y, [state["conn"]], raw_old)
            what.append("CRTC %d back on fb %d, %s" % (state["crtc"], fb_old, mode_text(mode_unpack(raw_old))))
        else:
            drm.set_crtc(state["crtc"], 0, 0, 0, [], None)
            what.append("CRTC %d off again, as found" % state["crtc"])
    except OSError as e:
        what.append("CRTC %d not given back (%s): the close leaves it to the kernel and fbdev"
                    % (state["crtc"], os.strerror(e.errno)))
    for f, arg, label in ((drm.rm_fb, state["fb"], "fb"), (drm.destroy_dumb, state["handle"], "buffer")):
        if not arg:
            continue
        try:
            f(arg)
        except OSError as e:
            what.append("%s %d not removed (%s)" % (label, arg, os.strerror(e.errno)))
    return "; ".join(what)


def hold(argv, ioctl=fcntl.ioctl):
    """The command. ioctl is the made-up device in the self-test."""
    opts = {"--card": "/dev/dri/card0", "--connector": "HDMI-A-1", "--max-hold": "600", "--log": ""}
    if len(argv) % 2:
        return usage()
    for k, v in zip(argv[::2], argv[1::2]):
        if k not in opts:
            return usage()
        opts[k] = v
    try:
        max_hold = int(opts["--max-hold"])
    except ValueError:
        return usage()
    if max_hold <= 0:
        return usage()
    name = opts["--connector"]
    try:
        fd = os.open(opts["--card"], os.O_RDWR | os.O_CLOEXEC)
    except OSError as e:
        return say(3, "REFUSED: cannot open %s: %s" % (opts["--card"], os.strerror(e.errno)))
    drm = Drm(fd, ioctl)
    try:
        state = take(drm, name)
    except Refused as e:
        return say(3, "REFUSED: %s; nothing was changed" % e)
    except SetFailed as e:
        return say(1, "FAILED: %s: %s" % (name, e))
    old = state["old"]
    was = ("fb %d, %s" % (old[0], mode_text(mode_unpack(old[4])))) if old[3] else "off"
    # Block the stop signals before the fork, so none is lost between the
    # fork and the wait.
    stops = {signal.SIGTERM, signal.SIGINT, signal.SIGHUP}
    signal.pthread_sigmask(signal.SIG_BLOCK, stops)
    sys.stdout.flush()
    pid = os.fork()
    if pid:
        # The parent: the child has the card open too, so this close is not
        # the last one and changes nothing.
        say(0, "t6-kms-mode: %s on CRTC %d: %s set (fb %d), read back; was %s; held by pid %d for at most %d s"
            % (name, state["crtc"], mode_text(mode_unpack(state["mode"])), state["fb"], was, pid, max_hold))
        os._exit(0)
    # The child: its own session (the ssh session that started it ends) and
    # no tie to the caller's stdio (ssh would wait for it otherwise).
    # Every other descriptor is closed too: one the caller handed down (a
    # pipe) would keep the caller waiting for this process.
    os.setsid()
    null = os.open(os.devnull, os.O_RDWR)
    for i in (0, 1, 2):
        os.dup2(null, i)
    os.closerange(3, fd)
    os.closerange(fd + 1, os.sysconf("SC_OPEN_MAX"))
    got = signal.sigtimedwait(stops, max_hold)
    why = "signal %s" % signal.Signals(got.si_signo).name if got else "max hold %d s" % max_hold
    done = give_back(drm, state)
    os.close(fd)
    if opts["--log"]:
        with open(opts["--log"], "a") as f:
            f.write("t6-kms-mode: pid %d: stop (%s): %s; card closed\n" % (os.getpid(), why, done))
    os._exit(0)


def say(rc, line):
    print(line if line.startswith("t6-kms-mode:") else "t6-kms-mode: " + line, flush=True)
    return rc


def usage():
    print("usage: t6-kms-mode.py hold [--card PATH] [--connector NAME] [--max-hold S] [--log FILE]\n"
          "       t6-kms-mode.py --self-test", file=sys.stderr)
    return 2


# ---- the self-test ------------------------------------------------------------

def self_test():
    import ctypes  # the self-test only: the made-up device writes the arrays

    count = [0]
    failed = []
    # The lines whose check has run. Every line of this file that calls
    # check must be in it at the end: a check that an early return or a
    # condition skipped fails the test, and no count is kept by hand.
    ran = set()

    def check(cond, what):
        count[0] += 1
        ran.add(sys._getframe(1).f_lineno)
        print("%s %s" % (what, "ok" if cond else "FAIL"))
        if not cond:
            failed.append(what)

    # The kernel's values on arm64, read from include/uapi/drm/drm.h and
    # drm_mode.h with aarch64-linux-gnu-gcc (sizeof and the DRM_IOCTL_*
    # macros). A layout or number off by one field would differ here.
    for got, want, what in (
            (IOCTL_SET_MASTER, 0x641E, "DRM_IOCTL_SET_MASTER"),
            (IOCTL_GETRESOURCES, 0xC04064A0, "DRM_IOCTL_MODE_GETRESOURCES"),
            (IOCTL_GETCRTC, 0xC06864A1, "DRM_IOCTL_MODE_GETCRTC"),
            (IOCTL_SETCRTC, 0xC06864A2, "DRM_IOCTL_MODE_SETCRTC"),
            (IOCTL_GETENCODER, 0xC01464A6, "DRM_IOCTL_MODE_GETENCODER"),
            (IOCTL_GETCONNECTOR, 0xC05064A7, "DRM_IOCTL_MODE_GETCONNECTOR"),
            (IOCTL_ADDFB, 0xC01C64AE, "DRM_IOCTL_MODE_ADDFB"),
            (IOCTL_RMFB, 0xC00464AF, "DRM_IOCTL_MODE_RMFB"),
            (IOCTL_CREATE_DUMB, 0xC02064B2, "DRM_IOCTL_MODE_CREATE_DUMB"),
            (IOCTL_DESTROY_DUMB, 0xC00464B4, "DRM_IOCTL_MODE_DESTROY_DUMB"),
            (MODEINFO.size, 68, "sizeof(struct drm_mode_modeinfo)"),
            (CARD_RES.size, 64, "sizeof(struct drm_mode_card_res)"),
            (GET_CONNECTOR.size, 80, "sizeof(struct drm_mode_get_connector)"),
            (GET_ENCODER.size, 20, "sizeof(struct drm_mode_get_encoder)"),
            (CRTC.size, 104, "sizeof(struct drm_mode_crtc)"),
            (CREATE_DUMB.size, 32, "sizeof(struct drm_mode_create_dumb)"),
            (FB_CMD.size, 28, "sizeof(struct drm_mode_fb_cmd)")):
        check(got == want, "uapi: %s = 0x%x (kernel 0x%x)" % (what, got, want))

    # MODE is the Rust MODE the image adopts with.
    rs = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                      "../../rust/src/board/nanopc_t6_display.rs")
    try:
        with open(rs) as f:
            text = f.read()
        body = text.split("pub const MODE: VideoMode = VideoMode {", 1)[1].split("};", 1)[0]
        v = {}
        for line in body.splitlines():
            line = line.strip().rstrip(",")
            if ":" in line:
                k, val = (s.strip() for s in line.split(":", 1))
                v[k] = val.replace("_", "")
        n = {k: int(v[k]) for k in ("pixel_clock_hz", "hactive", "hfront", "hsync", "hback",
                                    "vactive", "vfront", "vsync", "vback")}
        rust = {
            "clock": n["pixel_clock_hz"] // 1000,
            "hdisplay": n["hactive"], "hsync_start": n["hactive"] + n["hfront"],
            "hsync_end": n["hactive"] + n["hfront"] + n["hsync"],
            "htotal": n["hactive"] + n["hfront"] + n["hsync"] + n["hback"],
            "vdisplay": n["vactive"], "vsync_start": n["vactive"] + n["vfront"],
            "vsync_end": n["vactive"] + n["vfront"] + n["vsync"],
            "vtotal": n["vactive"] + n["vfront"] + n["vsync"] + n["vback"],
        }
        rust_flags = ((FLAG_PHSYNC if v["hsync_high"] == "true" else FLAG_NHSYNC)
                      | (FLAG_PVSYNC if v["vsync_high"] == "true" else FLAG_NVSYNC))
        same = rust == MODE and rust_flags == MODE_FLAGS and n["pixel_clock_hz"] % 1000 == 0
    except (OSError, IndexError, KeyError, ValueError) as e:
        rust, same = "unread (%s)" % e, False
    check(same, "MODE is board::nanopc_t6_display::MODE (%s)" % (rust,))

    def mk(clock=65000, h=(1024, 1048, 1184, 1344), v=(768, 771, 777, 806), flags=MODE_FLAGS,
           hskew=0, vscan=0, name="1024x768", vrefresh=60, mtype=0x40):
        return MODEINFO.pack(clock, h[0], h[1], h[2], h[3], hskew, v[0], v[1], v[2], v[3], vscan,
                             vrefresh, flags, mtype, name.encode())

    good = mk()
    check(mode_matches(mode_unpack(good)), "rule: the DMT 1024x768 at 60 Hz matches")
    check(mode_matches(mode_unpack(mk(name="other", vrefresh=59, mtype=0x48))),
          "rule: name, refresh and type bits do not matter")
    check(not mode_matches(mode_unpack(mk(clock=65001))), "rule: a clock of 65001 kHz does not match")
    check(not mode_matches(mode_unpack(mk(clock=75000))), "rule: 1024x768 at 70 Hz (75 MHz) does not match")
    check(not mode_matches(mode_unpack(mk(h=(1024, 1049, 1184, 1344)))), "rule: hsync_start off by one does not match")
    check(not mode_matches(mode_unpack(mk(v=(768, 771, 777, 807)))), "rule: vtotal off by one does not match")
    check(not mode_matches(mode_unpack(mk(flags=FLAG_PHSYNC | FLAG_NVSYNC))), "rule: a positive hsync does not match")
    check(not mode_matches(mode_unpack(mk(flags=FLAG_NHSYNC | FLAG_PVSYNC))), "rule: a positive vsync does not match")
    check(not mode_matches(mode_unpack(mk(flags=FLAG_NHSYNC))), "rule: no vsync flag does not match")
    check(not mode_matches(mode_unpack(mk(flags=MODE_FLAGS | FLAG_INTERLACE))), "rule: interlace does not match")
    check(not mode_matches(mode_unpack(mk(hskew=1))), "rule: a skew does not match")
    p1080 = mk(148500, (1920, 2008, 2052, 2200), (1080, 1084, 1089, 1125), FLAG_PHSYNC | FLAG_PVSYNC, name="1920x1080")
    p70 = mk(clock=75000, h=(1024, 1048, 1184, 1328), vrefresh=70)
    check(mode_pick([p1080, p70, good]) == good, "pick: the matching mode out of a list of three")
    check(mode_pick([p1080, p70]) is None, "pick: none out of a list without it")
    check(mode_pick([]) is None, "pick: none out of an empty list")

    # The made-up card reads and writes each field at the kernel's byte
    # offset (offsetof on the same headers, with gcc), not through the
    # Structs above: a field that Drm takes at the wrong index holds another
    # value here, and a check below fails.
    def u32(buf, off):
        return struct.unpack_from("<I", buf, off)[0]

    def u64(buf, off):
        return struct.unpack_from("<Q", buf, off)[0]

    def put(buf, off, value, fmt="I"):
        struct.pack_into("<" + fmt, buf, off, value)

    # struct drm_mode_modeinfo: clock 0, hdisplay 4, hsync_start 6,
    # hsync_end 8, htotal 10, hskew 12, vdisplay 14, vsync_start 16,
    # vsync_end 18, vtotal 20, vscan 22, vrefresh 24, flags 28, type 32,
    # name 36.
    raw = bytearray(MODEINFO.size)
    for off, fmt, value in ((0, "I", 65000), (4, "H", 1024), (6, "H", 1048), (8, "H", 1184), (10, "H", 1344),
                            (14, "H", 768), (16, "H", 771), (18, "H", 777), (20, "H", 806), (24, "I", 60),
                            (28, "I", MODE_FLAGS), (32, "I", 0x40)):
        put(raw, off, value, fmt)
    raw[36:44] = b"1024x768"
    check(bytes(raw) == good, "uapi: the mode written at the kernel's offsets is the mode the Struct packs")

    class Fake:
        """A made-up card: CRTC 81 drives HDMI-A-1 (connector 83, encoder 82)
        with fbdev's fb 50 at 1920x1080. It records each request by name.
        fail is (request, errno): that request is refused."""

        def __init__(self, modes, connection=CONNECTED, encoder=82, master_err=0,
                     setcrtc_err=0, readback=None, fail=None):
            self.modes, self.connection, self.encoder = modes, connection, encoder
            self.master_err, self.setcrtc_err, self.readback = master_err, setcrtc_err, readback
            self.fail = fail
            self.crtc = (50, 0, 0, 1, p1080)
            self.log = []

        def __call__(self, fd, req, buf, mutate=True):
            if self.fail and req == self.fail[0]:
                raise OSError(self.fail[1], os.strerror(self.fail[1]))
            if req == IOCTL_SET_MASTER:
                self.log.append("SET_MASTER")
                if self.master_err:
                    raise OSError(self.master_err, os.strerror(self.master_err))
                return 0
            if req == IOCTL_GETRESOURCES:
                # struct drm_mode_card_res: crtc_id_ptr 8, connector_id_ptr
                # 16, count_crtcs 36, count_connectors 40
                if u32(buf, 36) >= 1 and u32(buf, 40) >= 2:
                    ctypes.memmove(u64(buf, 8), U32.pack(81), 4)
                    ctypes.memmove(u64(buf, 16), struct.pack("<II", 40, 83), 8)
                put(buf, 36, 1)
                put(buf, 40, 2)
            elif req == IOCTL_GETCONNECTOR:
                # struct drm_mode_get_connector: modes_ptr 8, count_modes 32,
                # encoder_id 44, connector_id 48, connector_type 52,
                # connector_type_id 56, connection 60
                cid = u32(buf, 48)
                modes = self.modes if cid == 83 else [good]
                if u32(buf, 32) >= len(modes) and modes:
                    ctypes.memmove(u64(buf, 8), b"".join(modes), MODEINFO.size * len(modes))
                put(buf, 32, len(modes))
                put(buf, 44, self.encoder if cid == 83 else 0)
                put(buf, 52, 11 if cid == 83 else 14)
                put(buf, 56, 1)
                put(buf, 60, self.connection if cid == 83 else 2)
                self.log.append("GETCONNECTOR %d count %d" % (cid, len(modes)))
            elif req == IOCTL_GETENCODER:
                # struct drm_mode_get_encoder: encoder_id 0, crtc_id 8
                put(buf, 8, 81 if u32(buf, 0) == 82 else 0)
            elif req == IOCTL_GETCRTC:
                # struct drm_mode_crtc: set_connectors_ptr 0,
                # count_connectors 8, crtc_id 12, fb_id 16, x 20, y 24,
                # mode_valid 32, mode 36
                fb, x, y, valid, raw = self.crtc if u32(buf, 12) == 81 else (0, 0, 0, 0, bytes(MODEINFO.size))
                if self.readback and self.log and self.log[-1].startswith("SETCRTC 81 fb 60"):
                    fb, raw = self.readback
                for off, value in ((16, fb), (20, x), (24, y), (32, valid)):
                    put(buf, off, value)
                buf[36:36 + MODEINFO.size] = raw
                self.log.append("GETCRTC")
            elif req == IOCTL_SETCRTC:
                n = u32(buf, 8)
                conns = list(struct.unpack("<%dI" % n, ctypes.string_at(u64(buf, 0), 4 * n))) if n else []
                crtc, fb, x, y, valid = (u32(buf, off) for off in (12, 16, 20, 24, 32))
                raw = bytes(buf[36:36 + MODEINFO.size])
                m = mode_unpack(raw).name if valid else "none"
                self.log.append("SETCRTC %d fb %d at %d,%d conns %s mode %s" % (crtc, fb, x, y, conns, m))
                if self.setcrtc_err and fb == 60:
                    raise OSError(self.setcrtc_err, os.strerror(self.setcrtc_err))
                self.crtc = (fb, x, y, valid, raw)
            elif req == IOCTL_CREATE_DUMB:
                # struct drm_mode_create_dumb: height 0, width 4, bpp 8,
                # handle 16, pitch 20, size 24 (64 bits)
                h, w = u32(buf, 0), u32(buf, 4)
                self.log.append("CREATE_DUMB %dx%d bpp %d" % (w, h, u32(buf, 8)))
                put(buf, 16, 7)
                put(buf, 20, w * 4)
                put(buf, 24, w * 4 * h, "Q")
            elif req == IOCTL_ADDFB:
                # struct drm_mode_fb_cmd: fb_id 0, width 4, height 8, pitch
                # 12, bpp 16, depth 20, handle 24
                self.log.append("ADDFB %dx%d pitch %d bpp %d depth %d handle %d"
                                % tuple(u32(buf, off) for off in (4, 8, 12, 16, 20, 24)))
                put(buf, 0, 60)
            elif req == IOCTL_RMFB:
                self.log.append("RMFB %d" % U32.unpack(bytes(buf))[0])
            elif req == IOCTL_DESTROY_DUMB:
                self.log.append("DESTROY_DUMB %d" % U32.unpack(bytes(buf))[0])
            else:
                raise OSError(errno.ENOTTY, "unknown request 0x%x" % req)
            return 0

    def changes(log):
        return [e for e in log if e.split()[0] in ("SETCRTC", "CREATE_DUMB", "ADDFB", "RMFB", "DESTROY_DUMB")]

    fake = Fake([p1080, p70, good])
    drm = Drm(-1, fake)
    state = take(drm, "HDMI-A-1")
    check(changes(fake.log) == ["CREATE_DUMB 1024x768 bpp 32", "ADDFB 1024x768 pitch 4096 bpp 32 depth 24 handle 7",
                                "SETCRTC 81 fb 60 at 0,0 conns [83] mode 1024x768"],
          "take: a 1024x768 buffer, then SETCRTC 81 with it, connector 83 and the matching mode (%s)"
          % changes(fake.log))
    check(fake.log[0] == "SET_MASTER" and fake.log[-1] == "GETCRTC", "take: master first, the CRTC read back last")
    check(state["mode"] == good and state["old"][0] == 50 and state["old"][4] == p1080,
          "take: keeps the CRTC as found (fb 50, 1920x1080)")
    n = len(fake.log)
    done = give_back(drm, state)
    check(changes(fake.log[n:]) == ["SETCRTC 81 fb 50 at 0,0 conns [83] mode 1920x1080", "RMFB 60", "DESTROY_DUMB 7"],
          "give_back: CRTC 81 back on fb 50 at 1920x1080, then our fb and buffer away (%s)" % changes(fake.log[n:]))
    check(done.startswith("CRTC 81 back on fb 50"), "give_back: says so ('%s')" % done)

    def refused(fake, want):
        try:
            take(Drm(-1, fake), "HDMI-A-1")
        except Refused as e:
            return want in str(e) and not changes(fake.log), str(e)
        return False, "not refused"

    for fake, want, what in (
            (Fake([p1080, p70]), "lists no mode with the adopt's timing (2 modes; of size 1024x768: "
             '"1024x768" 75000 kHz', "a list without the mode"),
            (Fake([p1080, p70, good], connection=2), "is not connected", "a connector that is not connected"),
            (Fake([p1080, p70, good], encoder=0), "drives no CRTC", "a connector with no CRTC"),
            (Fake([p1080, p70, good], master_err=errno.EBUSY), "another master holds it", "another master"),
            (Fake([good], fail=(IOCTL_GETRESOURCES, errno.EOPNOTSUPP)),
             "cannot read the card's connectors (Operation not supported)", "a card without mode setting"),
            (Fake([good], fail=(IOCTL_GETCONNECTOR, errno.ENOENT)),
             "cannot read connector 40 (No such file or directory)", "a connector that cannot be read"),
            (Fake([good], fail=(IOCTL_GETENCODER, errno.ENOENT)),
             "cannot read encoder 82 of HDMI-A-1 (No such file or directory)", "an encoder that cannot be read"),
            (Fake([good], fail=(IOCTL_GETCRTC, errno.ENOENT)),
             "cannot read CRTC 81 (No such file or directory)", "a CRTC that cannot be read"),
            (Fake([good], fail=(IOCTL_CREATE_DUMB, errno.ENOMEM)),
             "cannot get a 1024x768 buffer from the card (Cannot allocate memory)", "no memory for the buffer")):
        ok, text = refused(fake, want)
        check(ok, "take: refuses %s and changes nothing (%s)" % (what, text))
    try:
        take(Drm(-1, Fake([good])), "HDMI-A-2")
        ok, text = False, "not refused"
    except Refused as e:
        ok, text = "no connector HDMI-A-2" in str(e), str(e)
    check(ok, "take: refuses a connector the card does not have (%s)" % text)

    # A set the kernel refuses, and one that reads back another mode: both
    # give the CRTC back before they report.
    fake = Fake([good], setcrtc_err=errno.EINVAL)
    try:
        take(Drm(-1, fake), "HDMI-A-1")
        ok = False
    except SetFailed as e:
        ok = str(e).startswith("Invalid argument; given back: CRTC 81 back on fb 50")
    check(ok and changes(fake.log)[-3:] == ["SETCRTC 81 fb 50 at 0,0 conns [83] mode 1920x1080", "RMFB 60", "DESTROY_DUMB 7"],
          "take: a SETCRTC the kernel refuses is given back (%s)" % changes(fake.log))
    fake = Fake([good], readback=(60, p70))
    try:
        take(Drm(-1, fake), "HDMI-A-1")
        ok = False
    except SetFailed as e:
        ok = "reads back fb 60" in str(e) and "given back: CRTC 81 back on fb 50" in str(e)
    check(ok and changes(fake.log)[-3:] == ["SETCRTC 81 fb 50 at 0,0 conns [83] mode 1920x1080", "RMFB 60", "DESTROY_DUMB 7"],
          "take: a read-back of another mode is given back (%s)" % changes(fake.log))
    # A CRTC that was off is switched off again.
    fake = Fake([good])
    fake.crtc = (0, 0, 0, 0, bytes(MODEINFO.size))
    drm = Drm(-1, fake)
    state = take(drm, "HDMI-A-1")
    n = len(fake.log)
    give_back(drm, state)
    check(changes(fake.log[n:])[0] == "SETCRTC 81 fb 0 at 0,0 conns [] mode none",
          "give_back: a CRTC found off is set off again (%s)" % changes(fake.log[n:]))

    # The command as a process, on the made-up card (/dev/null stands in for
    # the card's file): the caller's pipe ends when the parent exits (ssh
    # would wait for the holder otherwise), and the holder gives the CRTC
    # back on SIGTERM and by itself after --max-hold.
    import re
    import select
    import tempfile
    import time

    def run_hold(args, fake):
        r, w = os.pipe()
        sys.stdout.flush()  # or the child's copy of the buffer goes out again
        pid = os.fork()
        if pid == 0:
            os.close(r)
            os.dup2(w, 1)
            rc = 2
            try:
                rc = hold(["--card", os.devnull] + args, fake)
            finally:
                os._exit(rc)
        os.close(w)
        out, eof = b"", False
        while select.select([r], [], [], 10)[0]:
            b = os.read(r, 4096)
            if not b:
                eof = True
                break
            out += b
        os.close(r)
        rc = os.waitpid(pid, 0)[1]
        m = re.search(r"held by pid (\d+) ", out.decode())
        return os.waitstatus_to_exitcode(rc), out.decode().strip(), eof, int(m.group(1)) if m else 0

    def read(path):
        try:
            with open(path) as f:
                return f.read()
        except OSError:
            return ""

    def gone(pid, seconds):
        for _ in range(seconds * 10):
            try:
                with open("/proc/%d/stat" % pid) as f:
                    if f.read().rsplit(")", 1)[1].split()[0] == "Z":
                        return True
            except OSError:
                return True
            time.sleep(0.1)
        return False

    with tempfile.TemporaryDirectory() as d:
        log = os.path.join(d, "kms-mode.log")
        rc, out, eof, holder = run_hold(["--log", log, "--max-hold", "60"], Fake([p1080, good]))
        check(rc == 0 and eof and holder > 0 and out.startswith("t6-kms-mode: HDMI-A-1 on CRTC 81: \"1024x768\" 65000 kHz"),
              "hold: exit 0, one line naming the holder, and the caller's pipe ends (rc %d, eof %s: %s)" % (rc, eof, out))
        if holder:
            os.kill(holder, signal.SIGTERM)
        ended = holder > 0 and gone(holder, 5)
        if holder and not ended:
            os.kill(holder, signal.SIGKILL)
        text = read(log)
        check(ended and "stop (signal SIGTERM): CRTC 81 back on fb 50" in text and text.endswith("card closed\n"),
              "hold: SIGTERM gives the CRTC back and ends the holder (%s)" % text.strip())
        if os.path.exists(log):
            os.unlink(log)
        rc, out, eof, holder = run_hold(["--log", log, "--max-hold", "1"], Fake([p1080, good]))
        ended = holder > 0 and gone(holder, 5)
        text = read(log)
        check(rc == 0 and ended and "stop (max hold 1 s): CRTC 81 back on fb 50" in text,
              "hold: the holder gives the CRTC back by itself after --max-hold (%s)" % text.strip())
        if holder and not ended:
            os.kill(holder, signal.SIGKILL)
        if os.path.exists(log):
            os.unlink(log)
        rc, out, eof, holder = run_hold(["--log", log], Fake([p1080]))
        check(rc == 3 and eof and holder == 0 and not os.path.exists(log) and out.startswith("t6-kms-mode: REFUSED: HDMI-A-1 lists no mode")
              and out.endswith("; nothing was changed"), "hold: a refusal exits 3 with one line and leaves no holder (%s)" % out)
        rc, out, eof, holder = run_hold(["--log", log], Fake([good], fail=(IOCTL_GETRESOURCES, errno.EOPNOTSUPP)))
        check(rc == 3 and eof and holder == 0 and not os.path.exists(log)
              and out == "t6-kms-mode: REFUSED: cannot read the card's connectors (Operation not supported); nothing was changed",
              "hold: a read the card refuses exits 3 with one line, no traceback (%s)" % out)

    check(usage_rc(["hold", "--max-hold"]) == 2 and usage_rc(["hold", "--bogus", "1"]) == 2
          and usage_rc(["hold", "--max-hold", "0"]) == 2, "usage: a lone option, an unknown one and --max-hold 0 exit 2")

    # A call over several lines reports one of them, which one depends on
    # the Python version: any line of the call counts.
    import ast
    with open(os.path.abspath(__file__)) as f:
        calls = [(c.lineno, c.end_lineno) for c in ast.walk(ast.parse(f.read()))
                 if isinstance(c, ast.Call) and isinstance(c.func, ast.Name) and c.func.id == "check"]
    not_run = [a for a, b in calls if not ran.intersection(range(a, b + 1))]
    print("t6-kms-mode self-test: %d checks run, %d failed%s" % (
        count[0], len(failed), ", check lines not run: %s" % not_run if not_run else ""))
    return 0 if not failed and not not_run and count[0] else 1


def usage_rc(argv):
    stderr, sys.stderr = sys.stderr, open(os.devnull, "w")
    try:
        return main(["t6-kms-mode.py"] + argv)
    finally:
        sys.stderr.close()
        sys.stderr = stderr


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:2] == ["hold"]:
        return hold(argv[2:])
    return usage()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
