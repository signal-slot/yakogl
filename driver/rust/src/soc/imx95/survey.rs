// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A read-only survey of the System Manager (SM) over SCMI, for the
//! Verdin iMX95's first board session (tools/probes/verdin_imx95/imx95_scmi;
//! the display note's Z0, driver/docs/imx95-display.md section 6; the
//! pre-study's B2 SCMI half, docs/notes/pre-study-imx95.md section 6).
//!
//! It sends only messages that read ([`read_only`]; [`survey`] refuses to
//! send any other, and the host test checks the SM's log holds no write):
//! the base protocol's discovery (version, attributes, vendor, sub-vendor,
//! implementation version, the protocol list, the calling agent), every
//! listed protocol's version and attributes, then for the GPU's and the
//! display's resources the attributes and current state: power domains
//! [`POWER_DOMAINS`] (GPU 14, DISPLAY 13, CAMERA 3), performance domains
//! [`PERF_DOMAINS`] (GPU 9, DISP 12, CAM 11: attributes, levels, limits,
//! the level), clocks [`CLOCKS`] (attributes and name, rate, parent,
//! permissions, configuration).
//!
//! Each fact is one line, `<key> = <value>`, the key stable (a script
//! diffs the lines against tools/probes/verdin_imx95/imx95_scmi/expected.txt,
//! taken from Toradex's public SM configuration). A message the SM answers
//! with a non-zero status is a line too, the value the status's name and
//! number (`NOT_SUPPORTED (-1)`): a refusal is a fact, not a stop. Only a
//! dead transport stops the survey (the base version unanswered).

use super::sm::{self, Fault, LevelsFault, Resp};
use crate::firmware::scmi::{self, Transport};
use core::fmt::Write;

/// The power domains: GPU ([`super::gpu_power::PD_GPU`]), DISPLAY
/// ([`super::display_power::PD_DISPLAY`]), CAMERA ([`super::dsi_power::PD_CAMERA`]).
pub const POWER_DOMAINS: [u32; 3] = [14, 13, 3];
/// The performance domains: GPU 9, DISP 12, CAM 11.
pub const PERF_DOMAINS: [u32; 3] = [9, 12, 11];
/// The clocks the port uses or depends on, by SM id (dev_sm_clock.h; the
/// Zephyr `imx95_clock.h` values): the GPU's gate, root and APB (174, 84,
/// 83); the display's APB, AXI, OCRAM and pixel clocks (75, 76, 78, 80);
/// VIDEOPLL1's VCO and output (18, 19); the LDB PLL's VCO and output (37,
/// 38); CAMAPB (62); the MIPI PHY's cfg, PLL bypass, PLL reference and
/// test byte clocks (66..69); TPM6, the backlight (155); LPI2C2 and LPI2C3,
/// the expander's and the bridge's buses (48, 129).
pub const CLOCKS: [u32; 19] = [174, 84, 83, 75, 76, 78, 80, 18, 19, 37, 38, 62, 66, 67, 68, 69, 155, 48, 129];

/// The protocols queried even when the list does not name them: power,
/// performance, clock.
const ALWAYS: [u8; 3] = [scmi::PROTO_POWER, scmi::PROTO_PERF, scmi::PROTO_CLOCK];

// Message ids this survey sends that `firmware::scmi` does not name
// (DEN0056 3.2).
const BASE_DISCOVER_SUB_VENDOR: u8 = 0x4;
const BASE_DISCOVER_IMPLEMENTATION_VERSION: u8 = 0x5;
const BASE_DISCOVER_LIST_PROTOCOLS: u8 = 0x6;
const BASE_DISCOVER_AGENT: u8 = 0x7;
const CLOCK_CONFIG_GET: u8 = 0xb;
const CLOCK_GET_PERMISSIONS: u8 = 0xf;
/// DISCOVER_AGENT's id for the calling agent.
const AGENT_SELF: u32 = 0xffff_ffff;

/// True when message `msg` of protocol `proto` only reads (DEN0056 3.2):
/// PROTOCOL_VERSION and PROTOCOL_ATTRIBUTES of any protocol, the base
/// protocol's DISCOVER_*, and the GETs, ATTRIBUTES and DESCRIBEs of the
/// power, performance and clock protocols. Nothing else is ever sent.
pub const fn read_only(proto: u8, msg: u8) -> bool {
    if msg == scmi::PROTOCOL_VERSION || msg == scmi::PROTOCOL_ATTRIBUTES {
        return true;
    }
    match proto {
        scmi::PROTO_BASE => matches!(
            msg,
            scmi::BASE_DISCOVER_VENDOR
                | BASE_DISCOVER_SUB_VENDOR
                | BASE_DISCOVER_IMPLEMENTATION_VERSION
                | BASE_DISCOVER_LIST_PROTOCOLS
                | BASE_DISCOVER_AGENT
        ),
        scmi::PROTO_POWER => matches!(msg, scmi::POWER_DOMAIN_ATTRIBUTES | scmi::POWER_STATE_GET),
        scmi::PROTO_PERF => matches!(
            msg,
            scmi::PERF_DOMAIN_ATTRIBUTES | scmi::PERF_DESCRIBE_LEVELS | scmi::PERF_LIMITS_GET | scmi::PERF_LEVEL_GET
        ),
        scmi::PROTO_CLOCK => matches!(
            msg,
            scmi::CLOCK_ATTRIBUTES
                | scmi::CLOCK_DESCRIBE_RATES
                | scmi::CLOCK_RATE_GET
                | CLOCK_CONFIG_GET
                | scmi::CLOCK_PARENT_GET
                | CLOCK_GET_PERMISSIONS
        ),
        _ => false,
    }
}

/// What the survey counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Lines printed.
    pub facts: u32,
    /// Of them, the messages the SM answered with a non-zero status.
    pub refused: u32,
    /// Of them, the messages the transport or the reply's form failed.
    pub broken: u32,
}

/// Why the survey stopped before its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The base protocol's PROTOCOL_VERSION was not answered: the channel
    /// is dead; nothing after it was sent.
    NoBase,
}

/// One line, at most [`LINE_MAX`] bytes (a longer one is cut there).
pub const LINE_MAX: usize = 160;

struct Line {
    buf: [u8; LINE_MAX],
    n: usize,
}

impl Line {
    const fn new() -> Line {
        Line { buf: [0; LINE_MAX], n: 0 }
    }
    fn as_str(&self) -> &str {
        // Only `write_str` fills it, and it cuts at a char boundary.
        core::str::from_utf8(&self.buf[..self.n]).unwrap_or("")
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut take = s.len().min(LINE_MAX - self.n);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.n..self.n + take].copy_from_slice(&s.as_bytes()[..take]);
        self.n += take;
        Ok(())
    }
}

/// The 16-byte name at return values `at..at + 4` (NUL-padded ASCII);
/// a byte outside printable ASCII is `?`.
fn name(r: &Resp, at: usize, out: &mut Line) {
    for i in 0..scmi::NAME_LEN {
        let b = (r.get(at + i / 4) >> (8 * (i % 4))) as u8;
        if b == 0 {
            break;
        }
        let c = if (0x20..0x7f).contains(&b) { b as char } else { '?' };
        let _ = out.write_char(c);
    }
}

struct Survey<'a, T: Transport, F: FnMut(&str)> {
    t: &'a mut T,
    emit: F,
    counts: Counts,
}

impl<T: Transport, F: FnMut(&str)> Survey<'_, T, F> {
    /// One read-only message; `want` return values at least.
    fn get(&mut self, proto: u8, msg: u8, words: &[u32], want: usize) -> Result<Resp, Fault> {
        if !read_only(proto, msg) {
            // A programming error here, never a message on the channel.
            return Err(Fault::Transport(-1));
        }
        sm::call(self.t, proto, msg, words, want)
    }

    /// The line `key = value`, the value written by `value`.
    fn fact(&mut self, key: core::fmt::Arguments<'_>, value: impl FnOnce(&mut Line)) {
        let mut l = Line::new();
        let _ = l.write_fmt(key);
        let _ = l.write_str(" = ");
        value(&mut l);
        (self.emit)(l.as_str());
        self.counts.facts += 1;
    }

    /// The line for a message that did not answer as asked.
    fn fault(&mut self, key: core::fmt::Arguments<'_>, f: Fault) {
        match f {
            Fault::Refused(_) => self.counts.refused += 1,
            _ => self.counts.broken += 1,
        }
        self.fact(key, |l| write_fault(l, f));
    }

    /// `key = <value of r>` or the fault's line.
    fn one(
        &mut self,
        key: core::fmt::Arguments<'_>,
        r: Result<Resp, Fault>,
        value: impl FnOnce(&mut Line, &Resp),
    ) -> Option<Resp> {
        match r {
            Ok(r) => {
                self.fact(key, |l| value(l, &r));
                Some(r)
            }
            Err(f) => {
                self.fault(key, f);
                None
            }
        }
    }

    fn base(&mut self) -> Result<([u8; 32], usize), Stop> {
        let r = self.get(scmi::PROTO_BASE, scmi::PROTOCOL_VERSION, &[], 1);
        if self.one(format_args!("base version"), r, |l, r| hex(l, r.get(0))).is_none() {
            return Err(Stop::NoBase);
        }
        let r = self.get(scmi::PROTO_BASE, scmi::PROTOCOL_ATTRIBUTES, &[], 1);
        self.one(format_args!("base attributes"), r, |l, r| {
            let a = r.get(0);
            hex(l, a);
            let _ = write!(l, " ({} protocols, {} agents)", a & 0xff, a >> 8 & 0xff);
        });
        let r = self.get(scmi::PROTO_BASE, scmi::BASE_DISCOVER_VENDOR, &[], 4);
        self.one(format_args!("base vendor"), r, |l, r| name(r, 0, l));
        let r = self.get(scmi::PROTO_BASE, BASE_DISCOVER_SUB_VENDOR, &[], 4);
        self.one(format_args!("base sub-vendor"), r, |l, r| name(r, 0, l));
        let r = self.get(scmi::PROTO_BASE, BASE_DISCOVER_IMPLEMENTATION_VERSION, &[], 1);
        self.one(format_args!("base implementation"), r, |l, r| hex(l, r.get(0)));
        let r = self.get(scmi::PROTO_BASE, BASE_DISCOVER_AGENT, &[AGENT_SELF], 5);
        self.one(format_args!("base agent"), r, |l, r| {
            let _ = write!(l, "{} ", r.get(0));
            name(r, 1, l);
        });

        // the protocol list, from `skip` on until none remain
        let mut list = [0u8; 32];
        let mut n = 0usize;
        let mut fault = None;
        loop {
            match self.get(scmi::PROTO_BASE, BASE_DISCOVER_LIST_PROTOCOLS, &[n as u32], 1) {
                Ok(r) => {
                    let k = r.get(0) as usize;
                    if k == 0 || r.len() < 1 + k.div_ceil(4) {
                        break;
                    }
                    for i in 0..k {
                        if n < list.len() {
                            list[n] = (r.get(1 + i / 4) >> (8 * (i % 4))) as u8;
                            n += 1;
                        }
                    }
                    if n >= list.len() {
                        break;
                    }
                }
                Err(f) => {
                    fault = Some(f);
                    break;
                }
            }
        }
        match fault {
            Some(f) if n == 0 => self.fault(format_args!("base protocols"), f),
            _ => self.fact(format_args!("base protocols"), |l| {
                for (i, p) in list[..n].iter().enumerate() {
                    let _ = write!(l, "{}0x{:02x}", if i == 0 { "" } else { " " }, p);
                }
            }),
        }
        Ok((list, n))
    }

    fn protocols(&mut self, listed: &[u8]) {
        let mut all = [0u8; 35];
        let mut n = 0;
        for p in ALWAYS.iter().chain(listed) {
            if !all[..n].contains(p) {
                all[n] = *p;
                n += 1;
            }
        }
        all[..n].sort_unstable();
        for &p in &all[..n] {
            let r = self.get(p, scmi::PROTOCOL_VERSION, &[], 1);
            self.one(format_args!("proto 0x{:02x} version", p), r, |l, r| hex(l, r.get(0)));
            let r = self.get(p, scmi::PROTOCOL_ATTRIBUTES, &[], 1);
            self.one(format_args!("proto 0x{:02x} attributes", p), r, |l, r| hex(l, r.get(0)));
        }
    }

    fn power(&mut self) {
        for d in POWER_DOMAINS {
            let r = self.get(scmi::PROTO_POWER, scmi::POWER_DOMAIN_ATTRIBUTES, &[d], 5);
            self.one(format_args!("pd {} attributes", d), r, |l, r| {
                hex(l, r.get(0));
                let _ = l.write_str(" ");
                name(r, 1, l);
            });
            let r = self.get(scmi::PROTO_POWER, scmi::POWER_STATE_GET, &[d], 1);
            self.one(format_args!("pd {} state", d), r, |l, r| {
                let s = r.get(0);
                hex(l, s);
                let _ = l.write_str(match s {
                    scmi::POWER_ON => " (on)",
                    scmi::POWER_OFF => " (off)",
                    _ => " (other)",
                });
            });
        }
    }

    fn perf(&mut self) {
        for d in PERF_DOMAINS {
            let r = self.get(scmi::PROTO_PERF, scmi::PERF_DOMAIN_ATTRIBUTES, &[d], 8);
            self.one(format_args!("perf {} attributes", d), r, |l, r| {
                hex(l, r.get(0));
                let _ = write!(l, " rate limit {} us, sustained {} kHz level {}, ", r.get(1), r.get(2), r.get(3));
                name(r, 4, l);
            });
            match sm::describe_levels(self.t, d) {
                Ok((levels, count)) => self.fact(format_args!("perf {} levels", d), |l| {
                    for (i, v) in levels[..count].iter().enumerate() {
                        let _ = write!(l, "{}{}:{}", if i == 0 { "" } else { " " }, v.index, v.khz);
                    }
                }),
                Err(LevelsFault::Call(f)) => self.fault(format_args!("perf {} levels", d), f),
                Err(LevelsFault::Levels { returned, remaining }) => {
                    self.counts.broken += 1;
                    self.fact(format_args!("perf {} levels", d), |l| {
                        let _ = write!(l, "BAD REPLY ({} returned, {} remaining)", returned, remaining);
                    })
                }
            }
            let r = self.get(scmi::PROTO_PERF, scmi::PERF_LIMITS_GET, &[d], 2);
            self.one(format_args!("perf {} limits", d), r, |l, r| {
                let _ = write!(l, "max {} min {}", r.get(0), r.get(1));
            });
            let r = self.get(scmi::PROTO_PERF, scmi::PERF_LEVEL_GET, &[d], 1);
            self.one(format_args!("perf {} level", d), r, |l, r| {
                let _ = write!(l, "{}", r.get(0));
            });
        }
    }

    fn clocks(&mut self) {
        for c in CLOCKS {
            let r = self.get(scmi::PROTO_CLOCK, scmi::CLOCK_ATTRIBUTES, &[c], 5);
            self.one(format_args!("clk {} attributes", c), r, |l, r| {
                hex(l, r.get(0));
                let _ = l.write_str(" ");
                name(r, 1, l);
            });
            let r = self.get(scmi::PROTO_CLOCK, scmi::CLOCK_RATE_GET, &[c], 2);
            self.one(format_args!("clk {} rate", c), r, |l, r| {
                let _ = write!(l, "{}", u64::from(r.get(0)) | u64::from(r.get(1)) << 32);
            });
            let r = self.get(scmi::PROTO_CLOCK, scmi::CLOCK_PARENT_GET, &[c], 1);
            self.one(format_args!("clk {} parent", c), r, |l, r| {
                let _ = write!(l, "{}", r.get(0));
            });
            let r = self.get(scmi::PROTO_CLOCK, CLOCK_GET_PERMISSIONS, &[c], 1);
            self.one(format_args!("clk {} permissions", c), r, |l, r| {
                let p = r.get(0);
                hex(l, p);
                let _ = l.write_str(" (");
                let mut any = false;
                for (bit, what) in [(31, "state"), (30, "parent"), (29, "rate")] {
                    if p >> bit & 1 != 0 {
                        let _ = write!(l, "{}{}", if any { " " } else { "" }, what);
                        any = true;
                    }
                }
                let _ = l.write_str(if any { ")" } else { "none)" });
            });
            // flags 0: no extended configuration asked for
            let r = self.get(scmi::PROTO_CLOCK, CLOCK_CONFIG_GET, &[c, 0], 2);
            self.one(format_args!("clk {} config", c), r, |l, r| {
                let v = r.get(1);
                hex(l, v);
                let _ = l.write_str(if v & 1 != 0 { " (enabled)" } else { " (disabled)" });
            });
        }
    }
}

fn hex(l: &mut Line, v: u32) {
    let _ = write!(l, "0x{:08x}", v);
}

/// A fault as a value: the status's name and number, or what broke.
fn write_fault(l: &mut Line, f: Fault) {
    let _ = match f {
        Fault::Refused(s) => write!(l, "{} ({})", s.name(), s.code()),
        Fault::Transport(e) => write!(l, "TRANSPORT ERROR ({})", e),
        Fault::Echo { sent, got } => write!(l, "BAD REPLY (header 0x{:08x}, sent 0x{:08x})", got, sent),
        Fault::Length(n) => write!(l, "BAD REPLY ({} bytes)", n),
    };
}

/// The survey over `t`: every line to `emit` (without a newline), in the
/// order of the module documentation. Returns the counts, or why it stopped
/// (the counts so far in the error's companion).
pub fn survey<T: Transport>(t: &mut T, emit: impl FnMut(&str)) -> Result<Counts, (Stop, Counts)> {
    let mut s = Survey { t, emit, counts: Counts::default() };
    let (list, n) = match s.base() {
        Ok(v) => v,
        Err(e) => return Err((e, s.counts)),
    };
    s.protocols(&list[..n]);
    s.power();
    s.perf();
    s.clocks();
    Ok(s.counts)
}
