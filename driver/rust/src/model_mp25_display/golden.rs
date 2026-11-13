// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host harness (tests only): our display register state against a
//! `linux_display_dump` file. Plan #353, issues #7 and #1281.
//!
//! The dump is what `tools/probes/stm32mp257f_dk/linux_display_dump` reads from
//! ST's Linux (format in its README: `[regs]`, `[adv7535]`, `[edid]`,
//! `[modes]`, `[modetest]`). The allow-list
//! `driver/docs/stm32mp257f-dk-display-allow.txt` holds the read set (`#@read`
//! and `#@name` header lines) and, per register, the bits that may differ.
//!
//! [`ours`] runs our bring-up for the dump's mode (the preferred mode of the
//! dump's EDID through [`crate::video::edid_preferred`], else `HD720_60`) on
//! [`Mp25Display`] and [`Mp25I2c`] as the DK's image does (the sequence of the
//! model's own `dk_720p_four_lanes_breaks_no_rule` and of
//! `soc::stm32mp2::display_chain_tests`, copied: that module is not ours).
//! [`compare`] then reports every dumped register of the read set where
//! `(ours ^ linux) & !allowed != 0`. `ERR` values are skipped, as are
//! addresses outside the model's LTDC and DSI blocks (RCC, SYSCFG, RIFSC: the
//! model has none of them).
//!
//! Run the real comparison once `driver/docs/stm32mp257f-dk-display-dump-linux.txt`
//! exists:
//!
//!     cd driver/rust && cargo test -p yakogl-drivers --features stm32mp2 --lib -- --ignored linux_dump_matches

use super::{Mp25Display, DSI_BASE, DSI_LEN, LTDC_BASE, LTDC_LEN};
use crate::env::{Env, Level};
use crate::ip::adv7535::{self as adv7535, Bridge, Sink};
use crate::ip::dw_mipi_dsi::{self as dsi, flags, Format, Link};
use crate::ip::stm32_dsi_phy::{self as phy, StmDsiPhy};
use crate::ip::stm32_i2c::{self as i2c, Config as I2cConfig};
use crate::ip::stm32_ltdc as l;
use crate::mmio::Mmio;
use crate::model::TestEnv;
use crate::model_mp25_i2c::{Adv7535Config, Adv7535Handle, Mp25I2c, BASE as I2C2_BASE};
use crate::video::{self, VideoMode};
use std::collections::BTreeMap;

/// `FLEXGEN_28`, 27 MHz: the DSI PHY reference.
const REF_HZ: u32 = 27_000_000;
const LANES: u32 = 4;
const FB: u64 = 0x9000_0000;

// ------------------------------------------------------------------ parsers

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?, 16).ok()
}

/// One dumped register; `value` is `None` for `ERR`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpReg {
    pub name: String,
    pub value: Option<u32>,
}

/// The ADV7535's four register maps, offset to value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdvMaps {
    pub main: BTreeMap<u8, u8>,
    pub cec: BTreeMap<u8, u8>,
    pub edid: BTreeMap<u8, u8>,
    pub packet: BTreeMap<u8, u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Dump {
    pub regs: BTreeMap<u32, DumpReg>,
    pub adv: AdvMaps,
    /// The first non-empty `[edid]` line's bytes.
    pub edid: Vec<u8>,
}

pub fn parse_dump(text: &str) -> Result<Dump, String> {
    let mut d = Dump::default();
    let mut section = String::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let at = |m: &str| format!("line {}: {m}: {raw:?}", n + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            section = name.to_string();
            continue;
        }
        let w: Vec<&str> = line.split_whitespace().collect();
        match section.as_str() {
            "regs" => {
                if w.len() < 3 {
                    return Err(at("want ADDRESS VALUE NAME"));
                }
                let addr = parse_hex(w[0]).ok_or_else(|| at("bad address"))? as u32;
                let value =
                    if w[1] == "ERR" { None } else { Some(parse_hex(w[1]).ok_or_else(|| at("bad value"))? as u32) };
                d.regs.insert(addr, DumpReg { name: w[2].to_string(), value });
            }
            "adv7535" => {
                if w.len() < 3 {
                    return Err(at("want MAP OFFSET VALUE"));
                }
                let off = parse_hex(w[1]).ok_or_else(|| at("bad offset"))? as u8;
                // an ERR byte (an unreadable register) is left out of the map
                let Some(val) = parse_hex(w[2]) else { continue };
                let map = match w[0] {
                    "main" => &mut d.adv.main,
                    "cec" => &mut d.adv.cec,
                    "edid" => &mut d.adv.edid,
                    "packet" => &mut d.adv.packet,
                    _ => return Err(at("unknown map")),
                };
                map.insert(off, val as u8);
            }
            "edid" => {
                if d.edid.is_empty() && w.len() >= 2 {
                    let hex = w[1];
                    if hex.len() % 2 != 0 || !hex.is_ascii() {
                        return Err(at("bad EDID hex"));
                    }
                    d.edid = (0..hex.len() / 2)
                        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| at("bad EDID hex")))
                        .collect::<Result<_, _>>()?;
                }
            }
            _ => {}
        }
    }
    Ok(d)
}

/// The allow-list: the read set (address to name) and the masks of the bits
/// that may differ.
#[derive(Clone, Debug, Default)]
pub struct Allow {
    pub read: BTreeMap<u32, String>,
    pub mask: BTreeMap<u32, u32>,
}

pub fn parse_allow(text: &str) -> Result<Allow, String> {
    let mut a = Allow::default();
    let mut ranges: Vec<(u32, u32, String)> = Vec::new();
    let mut names: BTreeMap<u32, String> = BTreeMap::new();
    for (n, raw) in text.lines().enumerate() {
        let at = |m: &str| format!("line {}: {m}: {raw:?}", n + 1);
        let line = raw.trim();
        if let Some(r) = line.strip_prefix("#@read") {
            let w: Vec<&str> = r.split_whitespace().collect();
            if w.len() != 3 {
                return Err(at("want #@read ADDRESS WORDS NAME"));
            }
            let addr = parse_hex(w[0]).ok_or_else(|| at("bad address"))? as u32;
            let words: u32 = w[1].parse().map_err(|_| at("bad word count"))?;
            ranges.push((addr, words, w[2].to_string()));
        } else if let Some(r) = line.strip_prefix("#@name") {
            let w: Vec<&str> = r.split_whitespace().collect();
            if w.len() != 2 {
                return Err(at("want #@name ADDRESS NAME"));
            }
            names.insert(parse_hex(w[0]).ok_or_else(|| at("bad address"))? as u32, w[1].to_string());
        } else if line.is_empty() || line.starts_with('#') {
            continue;
        } else {
            let w: Vec<&str> = line.split_whitespace().collect();
            if w.len() < 3 {
                return Err(at("want ADDRESS MASK REASON"));
            }
            let addr = parse_hex(w[0]).ok_or_else(|| at("bad address"))? as u32;
            let mask = parse_hex(w[1]).ok_or_else(|| at("bad mask"))? as u32;
            a.mask.insert(addr, mask);
        }
    }
    for (base, words, name) in ranges {
        for i in 0..words {
            let addr = base + 4 * i;
            let nm = names.get(&addr).cloned().unwrap_or_else(|| format!("{name}+{:#x}", 4 * i));
            a.read.insert(addr, nm);
        }
    }
    Ok(a)
}

// -------------------------------------------------------------- our bring-up

/// The shim's job: bus ids of `ip::adv7535` to the DK's I2C addresses.
struct DkEnv<'a>(&'a Mp25I2c);

fn addr_of(bus: u32) -> u8 {
    match bus {
        adv7535::BUS_MAIN => 0x3d,
        adv7535::BUS_DSI_CEC => 0x3c,
        adv7535::BUS_EDID => 0x3f,
        adv7535::BUS_PACKET => 0x38,
        _ => panic!("bus {bus}"),
    }
}

impl Env for DkEnv<'_> {
    fn busy_wait_us(&self, _us: u32) {}
    fn sleep_ms(&self, _ms: u32) {}
    fn log_str(&self, _l: Level, _line: &str) {}
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
        i2c::write(self.0, I2C2_BASE, addr_of(bus), buf)
    }
    fn i2c_write_read(&self, bus: u32, w: &[u8], r: &mut [u8]) -> Result<(), i32> {
        i2c::write_read(self.0, I2C2_BASE, addr_of(bus), w, r)
    }
    fn gpio_set(&self, _l: u32, _v: bool) -> Result<(), i32> {
        Ok(())
    }
}

/// The state after our bring-up.
pub struct Ours {
    pub mode: VideoMode,
    pub disp: Mp25Display,
    pub adv: Adv7535Handle,
    pub edid: Vec<u8>,
}

/// The mode of an EDID, 720p60 when there is none or it does not parse.
pub fn mode_of(edid: &[u8]) -> VideoMode {
    video::edid_preferred(edid).unwrap_or(video::HD720_60)
}

/// LTDC, DSI host and PHY, and the ADV7535 started for `mode`, on the models;
/// `edid` is what the ADV7535's EDID map serves.
pub fn bring_up(mode: VideoMode, edid: Vec<u8>) -> Ours {
    let disp = Mp25Display::new();
    let env = TestEnv::default();
    let link = Link { mode, lanes: LANES, channel: 0, format: Format::Rgb888, flags: flags::ADV7535 };
    // pre_enable: the host and the PHY up in command mode
    dsi::probe(&disp, DSI_BASE).unwrap();
    phy::probe(&disp, &env, DSI_BASE).unwrap();
    let mut p = StmDsiPhy::new(DSI_BASE, REF_HZ);
    dsi::attach(&disp, &env, DSI_BASE, &mut p, &link).unwrap();
    // the CRTC: timings, the layer, the enables
    let ltdc = l::Ltdc::probe(&disp, LTDC_BASE).unwrap();
    let layer = l::Layer { phys: FB, pitch: 0, format: l::Format::Xrgb8888 };
    ltdc.configure(&disp, &mode, &l::Output::default(), &layer).unwrap();
    ltdc.enable(&disp, &env).unwrap();
    dsi::video_on(&disp, DSI_BASE, &link);
    assert_eq!(disp.violations(), Vec::<String>::new(), "our bring-up breaks an ordering rule");

    // the ADV7535 over I2C2 (100 MHz kernel clock, 400 kHz, rise 108 ns, fall 12 ns)
    let (bus, adv) = Mp25I2c::dk(Adv7535Config { edid: edid.clone(), ..Adv7535Config::default() });
    let mut c = I2cConfig::new(100_000_000, 400_000);
    c.rise_ns = 108;
    c.fall_ns = 12;
    i2c::init(&bus, I2C2_BASE, &c).unwrap();
    let denv = DkEnv(&bus);
    let b = Bridge::new(&denv);
    let acfg = adv7535::Config::new(LANES as u8);
    b.init(&acfg).unwrap();
    let vic = if mode.hactive == 1280 { 4 } else { 16 };
    b.start(&acfg, &mode, &Sink { hdmi: true, vic, aspect_16_9: true, refresh_hz: 60 }).unwrap();
    Ours { mode, disp, adv, edid }
}

/// Our bring-up for the dump's mode: its EDID's preferred one, else 720p60.
pub fn ours(dump: &Dump) -> Ours {
    if dump.edid.is_empty() {
        bring_up(video::HD720_60, Adv7535Config::default().edid)
    } else {
        bring_up(mode_of(&dump.edid), dump.edid.clone())
    }
}

/// Our value of a register, `None` outside the model's two blocks.
pub fn our_reg(m: &Mp25Display, addr: u32) -> Option<u32> {
    let a = addr as usize;
    let inside = (DSI_BASE..DSI_BASE + DSI_LEN).contains(&a) || (LTDC_BASE..LTDC_BASE + LTDC_LEN).contains(&a);
    inside.then(|| m.read32(a))
}

// --------------------------------------------------------------- comparison

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub addr: u32,
    pub name: String,
    pub ours: u32,
    pub linux: u32,
    /// The bits allowed to differ (0 when the register is not in the list).
    pub mask: u32,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:#010x} {}: ours {:#010x} linux {:#010x} differ {:#010x} (allowed {:#010x})",
            self.addr,
            self.name,
            self.ours,
            self.linux,
            (self.ours ^ self.linux) & !self.mask,
            self.mask
        )
    }
}

/// Every register of the allow-list's read set that the dump has (not `ERR`)
/// and the model reaches, with a bit differing outside its mask.
pub fn compare(ours: &Mp25Display, dump: &Dump, allow: &Allow) -> Vec<Mismatch> {
    let mut out = Vec::new();
    for (&addr, listed) in &allow.read {
        let Some(DumpReg { name, value: Some(linux) }) = dump.regs.get(&addr) else { continue };
        let Some(ours) = our_reg(ours, addr) else { continue };
        let mask = allow.mask.get(&addr).copied().unwrap_or(0);
        if (ours ^ linux) & !mask != 0 {
            let name = if name.is_empty() { listed.clone() } else { name.clone() };
            out.push(Mismatch { addr, name, ours, linux: *linux, mask });
        }
    }
    out
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_mp25_i2c::{edid_block, EdidMode};

    const ALLOW: &str = include_str!("../../../docs/stm32mp257f-dk-display-allow.txt");

    /// A dump as Linux's script would write it, from our own final state.
    fn synthetic(o: &Ours, allow: &Allow) -> String {
        let mut s = String::from("# linux_display_dump\n# kernel synthetic\n[regs]\n");
        for (&addr, name) in &allow.read {
            match our_reg(&o.disp, addr) {
                Some(v) => s += &format!("{addr:#010x} {v:#010x} {name}\n"),
                None => s += &format!("{addr:#010x} ERR {name}\n"),
            }
        }
        s += "[adv7535]\n";
        for off in 0..=255u8 {
            s += &format!("main {off:#04x} {:#04x}\n", o.adv.reg(off));
            s += &format!("cec {off:#04x} {:#04x}\n", o.adv.cec(off));
            s += &format!("packet {off:#04x} {:#04x}\n", o.adv.packet(off));
        }
        for (off, b) in o.edid.iter().enumerate().take(256) {
            s += &format!("edid {off:#04x} {b:#04x}\n");
        }
        let hex: String = o.edid.iter().map(|b| format!("{b:02x}")).collect();
        s += &format!("[edid]\nHDMI-A-1 {hex}\n[modes]\nHDMI-A-1 status connected\n[modetest]\nraw\n");
        s
    }

    /// `text` with the `[regs]` line of `name` replaced by `addr value name`.
    fn with_reg(text: &str, addr: u32, name: &str, value: &str) -> String {
        let line = text.lines().find(|l| l.split_whitespace().nth(2) == Some(name)).unwrap();
        text.replacen(line, &format!("{addr:#010x} {value} {name}"), 1)
    }

    fn setup(edid: Vec<u8>) -> (Ours, Allow) {
        (bring_up(mode_of(&edid), edid), parse_allow(ALLOW).unwrap())
    }

    #[test]
    fn allow_list_parses() {
        let a = parse_allow(ALLOW).unwrap();
        assert_eq!(a.read[&0x4801_0000], "LTDC_IDR");
        assert_eq!(a.read[&0x4801_0010], "LTDC_AWCR");
        assert_eq!(a.read[&0x4800_0400], "DSI_WCFGR");
        assert_eq!(a.read[&0x4801_0140], "LTDC_L1+0x40", "a word without #@name");
        assert_eq!(a.mask[&0x4801_0038], 0xffff_ffff);
        assert_eq!(a.mask[&0x4800_00b0], 0xffff_fffe);
        assert!(a.read.contains_key(&0x4208_0010), "RIFSC_SECCFGR");
    }

    #[test]
    fn dump_parses() {
        let d = parse_dump(
            "# c\n[regs]\n0x48010000 0x00040101 LTDC_IDR\n0x48010004 ERR LTDC_LCR\n[adv7535]\nmain 0x00 0x14\ncec 0x1c 0x40\nedid 0x01 0xff\npacket 0x52 0x10\n[edid]\nHDMI-A-1 00ff\n[modes]\nHDMI-A-1 status connected\n",
        )
        .unwrap();
        assert_eq!(d.regs[&0x4801_0000], DumpReg { name: "LTDC_IDR".into(), value: Some(0x40101) });
        assert_eq!(d.regs[&0x4801_0004].value, None);
        assert_eq!((d.adv.main[&0], d.adv.cec[&0x1c], d.adv.edid[&1], d.adv.packet[&0x52]), (0x14, 0x40, 0xff, 0x10));
        assert_eq!(d.edid, [0x00, 0xff]);
        assert!(parse_dump("[regs]\n0x1 0x2\n").is_err());
    }

    #[test]
    fn synthetic_dump_of_our_own_state_compares_clean() {
        for m in [EdidMode::P720, EdidMode::P1080] {
            let (o, allow) = setup(edid_block(m));
            let d = parse_dump(&synthetic(&o, &allow)).unwrap();
            assert_eq!(d.edid, o.edid);
            assert_eq!(d.adv.main[&0x00], o.adv.reg(0x00));
            assert!(d.regs.len() > 100);
            // the dump's mode is the one the harness brings up
            let again = ours(&d);
            assert_eq!(again.mode, o.mode);
            let mm = compare(&again.disp, &d, &allow);
            assert!(mm.is_empty(), "{}", mm.iter().map(|m| m.to_string()).collect::<Vec<_>>().join("\n"));
        }
    }

    #[test]
    fn a_flipped_non_allowed_bit_is_exactly_one_mismatch() {
        let (o, allow) = setup(edid_block(EdidMode::P720));
        let text = synthetic(&o, &allow);
        let awcr = 0x4801_0010;
        assert_eq!(allow.mask.get(&awcr), None);
        let v = our_reg(&o.disp, awcr).unwrap();
        let t = with_reg(&text, awcr, "LTDC_AWCR", &format!("{:#010x}", v ^ 1));
        let mm = compare(&o.disp, &parse_dump(&t).unwrap(), &allow);
        assert_eq!(mm, [Mismatch { addr: awcr, name: "LTDC_AWCR".into(), ours: v, linux: v ^ 1, mask: 0 }]);

        // bits inside an allowed mask are not reported
        let t = with_reg(&t, 0x4801_0038, "LTDC_ISR", "0xdeadbeef");
        assert_eq!(compare(&o.disp, &parse_dump(&t).unwrap(), &allow).len(), 1);
        // the PHY lock bit is outside DSI_PSR's mask (0xfffffffe)
        let psr = 0x4800_00b0;
        let v = our_reg(&o.disp, psr).unwrap();
        let t = with_reg(&t, psr, "DSI_PSR", &format!("{:#010x}", v ^ 0x1));
        let mm = compare(&o.disp, &parse_dump(&t).unwrap(), &allow);
        assert_eq!(mm.iter().map(|m| m.addr).collect::<Vec<_>>(), [psr, awcr]);
    }

    #[test]
    fn err_values_and_unmodelled_blocks_are_skipped() {
        let (o, allow) = setup(edid_block(EdidMode::P720));
        let text = synthetic(&o, &allow);
        let t = with_reg(&text, 0x4801_0010, "LTDC_AWCR", "ERR");
        assert!(compare(&o.disp, &parse_dump(&t).unwrap(), &allow).is_empty());
        // RCC is not modelled: any value is skipped
        let t = with_reg(&text, 0x4420_0840, "RCC_LTDCCFGR", "0x12345678");
        assert!(compare(&o.disp, &parse_dump(&t).unwrap(), &allow).is_empty());
        assert_eq!(our_reg(&o.disp, 0x4420_0840), None);
    }

    /// The real comparison. Needs `driver/docs/stm32mp257f-dk-display-dump-linux.txt`
    /// (`tools/probes/stm32mp257f_dk/linux_display_dump/run_dump.sh`). Run:
    ///
    ///     cargo test -p yakogl-drivers --features stm32mp2 --lib -- --ignored linux_dump_matches
    #[test]
    #[ignore = "needs driver/docs/stm32mp257f-dk-display-dump-linux.txt from the board"]
    fn linux_dump_matches() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/stm32mp257f-dk-display-dump-linux.txt");
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("linux_dump_matches: {path} is missing; nothing compared");
            return;
        };
        let dump = parse_dump(&text).expect("the dump parses");
        let allow = parse_allow(ALLOW).expect("the allow-list parses");
        let o = ours(&dump);
        let mm = compare(&o.disp, &dump, &allow);
        for m in &mm {
            eprintln!("MISMATCH {m}");
        }
        assert!(
            mm.is_empty(),
            "{} register(s) differ from Linux's (mode {}x{})",
            mm.len(),
            o.mode.hactive,
            o.mode.vactive
        );
    }
}
