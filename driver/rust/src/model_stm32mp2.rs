// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the STM32MP257F-DK GPU power path (tests only):
//! the RCC's GPU register, the GPU's identity, idle and interrupt registers,
//! and a scripted OP-TEE SCMI server for the power-domain and performance
//! protocols, at fake addresses.
//!
//! Like [`crate::model::Model`] and [`crate::model_am62p::Am62pModel`] it
//! records every access in order and answers only what the drivers poll.
//! Offsets, bits and values are restated here from their sources, not shared
//! with the drivers, so a wrong constant cannot agree with itself.
//!
//! # Where each constant comes from
//!
//! None of the RCC facts below is confirmed by RM0457, the STM32MP25
//! reference manual: it could not be fetched. They are cross-checked in open
//! sources only. The file:line citations of the studies r-galcore (ST's
//! galcore 6.4.21 platform code) and r-rcc-src (TF-A, OP-TEE, Zephyr, U-Boot
//! and Linux) were not available to the writer of this module (the study
//! reports are not in the tree and the dependency directory of
//! optee-panic did not exist on the node); they are quoted as the note
//! `docs/notes/pre-study-stm32mp257f-dk.md` carries them (section 4,
//! "Power, clocks, reset" and the Q3 row of section 7), and every one of them
//! is to be re-read with `git show` before it is cited elsewhere.
//!
//! - `RCC_GPUCFGR` at offset 0x83C of the RCC, **bit 1 gates both GPU clocks**
//!   (`CK_BUS_GPU` 91 and `CK_KER_GPU` 340 are one gate), **bit 0 is the GPU
//!   reset** (`GPU_R` 75): r-galcore (galcore 6.4.21 platform code), r-rcc-src
//!   (the open sources, cross-checked); DT ids in ST's Linux tree,
//!   `stm32mp255.dtsi:173-176`, `include/dt-bindings/clock/st,stm32mp25-rcc.h:114`
//!   and `:362`, `include/dt-bindings/reset/st,stm32mp25-rcc.h:85`.
//!   The reset's polarity (1 = asserted) and the register's reset value are
//!   **not known**; the model takes the stricter case: the register comes up
//!   with the clock gated and the reset **asserted** (0x1), so a driver that
//!   never releases the reset faults. [`Mp2Model::with_rcc_reset`] gives the
//!   other value. Bits other than 1:0 are reserved; writing them is a fault.
//! - The GPU registers (window 0x800 at 0x48280000, `stm32mp255.dtsi:171`,
//!   #1281): HI_CLOCK_CONTROL 0x00, HI_IDLE_STATE 0x04, HI_INTR_ACKNOWLEDGE
//!   0x10 (read once, acknowledges, zero means not ours: r-galcore),
//!   HI_INTR_ENBL 0x14, HI_CHIP_IDENTITY 0x18, HI_CHIP_FEATURE 0x1c,
//!   HI_CHIP_MODEL **0x20**, HI_CHIP_REV **0x24**, HI_CHIP_CUSTOMER_ID 0x30,
//!   HI_CHIP_PRODUCT_ID 0xa8, HI_CHIP_ECO_ID 0xe8 (etnaviv's `state_hi.xml.h`,
//!   the same names `gpu/vivante/regs.rs` cites). The task text put the
//!   model at 0x18 and the revision at 0x20; etnaviv and `regs.rs` have 0x18
//!   as the old-style identity word, which this model reads 0 (the model
//!   register is the one that answers on this part).
//!   Values read on the DK under ST's Linux (#1281): model 0x8000, revision
//!   0x6205, product 0x80003, eco 0; the customer id was **not read** and is
//!   modelled 0.
//! - SCMI: the Arm SCMI v3.x specification (power domain protocol 0x11,
//!   performance domain protocol 0x13: message ids, payloads, status codes);
//!   the ids and status codes are those of [`crate::firmware::scmi`], whose
//!   types this module builds on. The GPU is power domain 0 (`PD_SCMI_GPU`,
//!   `include/dt-bindings/power/st,stm32mp25-power.h:9`) and performance
//!   domain 1 (`plat-stm32mp2/scmi_server_scpfw.c:360-381`, optee_os
//!   `4.0.0-stm32mp-r3.1`; the CPU's is domain 0), from the note's section 4.
//! - A level change is **refused with DENIED while the GPU is powered**
//!   (`core/drivers/stm32_gpu_opp.c:50-58`, "Change OPP not allowed when GPU
//!   is used"): the model refuses it while power domain 0 is on, whatever the
//!   level asked for.
//!
//! # What is a fault
//!
//! A fault is recorded in the access log ([`Access::Fault`]), the access gets
//! no effect, a read is answered [`POISON`], and the test asserts
//! [`Mp2Model::faults`] is empty. The reason a wrong access matters is that
//! OP-TEE 4.0 panicked on the board when etnaviv loaded; the cause is **not
//! established** (note, "The etnaviv panic and what our image avoids"; the
//! optee-panic report was not readable here), so the conditions are the
//! strict reading of it and of the order r-galcore reports, each one
//! stricter than the board is known to need:
//! - [`FaultKind::GpuDomainOff`], [`FaultKind::GpuClockGated`],
//!   [`FaultKind::GpuInReset`]: any GPU register access while power domain 0
//!   is off, the `RCC_GPUCFGR` clock gate is closed or its reset is asserted,
//!   checked in that order. (A bus access to an unpowered or ungated block
//!   is the candidate cause of the panic.)
//! - [`FaultKind::GpuOutsideWindow`]: any access at or above 0x800 (#1281,
//!   r-galcore: "never read at or above it"), or unaligned.
//! - [`FaultKind::RccWhileDomainOff`]: a write to `RCC_GPUCFGR` while power
//!   domain 0 is off (the note: "never the clock gates before the domain").
//!   Reads of it are allowed.
//! - [`FaultKind::DomainOffWhileClockOn`]: power domain 0 turned off while
//!   the clock gate is open (teardown must close the gate first).
//! - [`FaultKind::RccReservedBits`], [`FaultKind::RccOtherRegister`]: a
//!   write of bits other than 1:0 to `RCC_GPUCFGR`, or any access to another
//!   RCC offset (this model knows no other).
//!
//! # The SCMI server
//!
//! [`ScmiServer`] implements [`Transport`], so a [`crate::firmware::scmi::Client`]
//! (or anything that takes the trait) runs against it unchanged. Every
//! command and its response are kept byte for byte ([`Mp2Model::messages`]).
//! Answers follow the specification; a status can be scripted per
//! (protocol, message) with [`ScmiServer::fail_next`] / [`ScmiServer::fail_always`]
//! (SUCCESS aside, DENIED and NOT_FOUND are the useful ones); a scripted
//! failure answers header and status only and changes nothing. Placeholders
//! the board has not given: the performance domain's levels ([`PERF_LEVEL_INIT`])
//! and attributes, the protocol versions (3.0).

use crate::firmware::scmi::{self, Transport};
use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The fake base of the RCC.
pub const RCC_BASE: usize = 0x1000_0000;
/// The fake base of the GPU.
pub const GPU_BASE: usize = 0x2000_0000;
/// The GPU's register window (#1281: size 0x800).
pub const GPU_WINDOW: u32 = 0x800;
/// What a faulting read returns.
pub const POISON: u32 = 0xdead_fa17;

/// `RCC_GPUCFGR`.
pub const RCC_GPUCFGR_OFF: u32 = 0x83c;
const GPUCFGR_RESET: u32 = 1 << 0;
const GPUCFGR_CLOCK: u32 = 1 << 1;

const HI_CLOCK_CONTROL: u32 = 0x00;
const HI_IDLE_STATE: u32 = 0x04;
const HI_INTR_ACKNOWLEDGE: u32 = 0x10;
const HI_INTR_ENBL: u32 = 0x14;
const HI_CHIP_IDENTITY: u32 = 0x18;
const HI_CHIP_MODEL: u32 = 0x20;
const HI_CHIP_REV: u32 = 0x24;
const HI_CHIP_CUSTOMER_ID: u32 = 0x30;
const HI_CHIP_PRODUCT_ID: u32 = 0xa8;
const HI_CHIP_ECO_ID: u32 = 0xe8;
const CLOCK_IDLE_3D: u32 = 1 << 16;
const CLOCK_IDLE_2D: u32 = 1 << 17;
const CLOCK_SOFT_RESET: u32 = 1 << 12;
const IDLE_ALL: u32 = 0x7fff_ffff;

/// The identity the board reported (#1281).
pub const MODEL: u32 = 0x8000;
pub const REVISION: u32 = 0x6205;
pub const PRODUCT_ID: u32 = 0x80003;
pub const ECO_ID: u32 = 0;

/// The GPU's power domain and performance domain ids.
pub const PD_GPU: u32 = 0;
pub const PERF_GPU: u32 = 1;
/// The performance level the GPU domain starts at: a placeholder, the board's
/// value was not read.
pub const PERF_LEVEL_INIT: u32 = 800_000;
const PERF_DOMAINS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    GpuDomainOff,
    GpuClockGated,
    GpuInReset,
    GpuOutsideWindow,
    RccWhileDomainOff,
    RccReservedBits,
    RccOtherRegister,
    DomainOffWhileClockOn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    /// The register offset (0 for the SCMI power-off fault).
    pub off: u32,
    pub write: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    RccR(u32, u32),
    RccW(u32, u32),
    GpuR(u32, u32),
    GpuW(u32, u32),
    /// SCMI power domain `.0` set to state `.1` (as the server accepted it).
    PowerSet(u32, u32),
    /// SCMI performance level of domain `.0` set to `.1`.
    LevelSet(u32, u32),
    /// The access before this entry was a fault.
    Fault(Fault),
}

/// One SCMI exchange as the server saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub tx: Vec<u8>,
    pub rx: Vec<u8>,
}

struct Shared {
    rcc: u32,
    domain_on: bool,
    levels: [u32; PERF_DOMAINS],
    gpu: BTreeMap<u32, u32>,
    intr_pending: u32,
    log: Vec<Access>,
    msgs: Vec<Message>,
    /// (protocol, message, status, remaining; None = always)
    script: Vec<(u8, u8, i32, Option<u32>)>,
}

/// The model: RCC, GPU and the power state the SCMI server drives.
pub struct Mp2Model(Rc<RefCell<Shared>>);

/// The SCMI server on the other side of the transport trait.
pub struct ScmiServer(Rc<RefCell<Shared>>);

impl Mp2Model {
    pub fn new() -> Mp2Model {
        Mp2Model::with_rcc_reset(GPUCFGR_RESET)
    }

    /// With another `RCC_GPUCFGR` reset value (see the module text).
    pub fn with_rcc_reset(v: u32) -> Mp2Model {
        Mp2Model(Rc::new(RefCell::new(Shared {
            rcc: v,
            domain_on: false,
            levels: [0, PERF_LEVEL_INIT],
            gpu: BTreeMap::new(),
            intr_pending: 0,
            log: Vec::new(),
            msgs: Vec::new(),
            script: Vec::new(),
        })))
    }

    /// The SCMI server sharing this model's power state.
    pub fn scmi(&self) -> ScmiServer {
        ScmiServer(self.0.clone())
    }

    pub fn rcc_base(&self) -> usize {
        RCC_BASE
    }

    pub fn gpu_base(&self) -> usize {
        GPU_BASE
    }

    pub fn accesses(&self) -> Vec<Access> {
        self.0.borrow().log.clone()
    }

    pub fn faults(&self) -> Vec<Fault> {
        self.0
            .borrow()
            .log
            .iter()
            .filter_map(|a| match a {
                Access::Fault(f) => Some(*f),
                _ => None,
            })
            .collect()
    }

    pub fn messages(&self) -> Vec<Message> {
        self.0.borrow().msgs.clone()
    }

    pub fn clear_log(&self) {
        let mut s = self.0.borrow_mut();
        s.log.clear();
        s.msgs.clear();
    }

    pub fn domain_on(&self) -> bool {
        self.0.borrow().domain_on
    }

    pub fn level(&self, perf_domain: usize) -> u32 {
        self.0.borrow().levels[perf_domain]
    }

    pub fn rcc(&self) -> u32 {
        self.0.borrow().rcc
    }

    pub fn clock_on(&self) -> bool {
        self.0.borrow().rcc & GPUCFGR_CLOCK != 0
    }

    pub fn in_reset(&self) -> bool {
        self.0.borrow().rcc & GPUCFGR_RESET != 0
    }

    /// The GPU raises its interrupt: `HI_INTR_ACKNOWLEDGE` shows `vec` until
    /// it is read.
    pub fn raise_interrupt(&self, vec: u32) {
        self.0.borrow_mut().intr_pending |= vec;
    }

    /// A GPU register as stored (nothing logged); None for the identity.
    pub fn peek(&self, off: u32) -> u32 {
        self.0.borrow().gpu.get(&off).copied().unwrap_or(0)
    }
}

impl Default for Mp2Model {
    fn default() -> Self {
        Mp2Model::new()
    }
}

impl Shared {
    fn fault(&mut self, kind: FaultKind, off: u32, write: bool) {
        self.log.push(Access::Fault(Fault { kind, off, write }));
    }

    /// The fault condition of a GPU access, if any.
    fn gpu_fault(&self, off: u32) -> Option<FaultKind> {
        if off >= GPU_WINDOW || off % 4 != 0 {
            Some(FaultKind::GpuOutsideWindow)
        } else if !self.domain_on {
            Some(FaultKind::GpuDomainOff)
        } else if self.rcc & GPUCFGR_CLOCK == 0 {
            Some(FaultKind::GpuClockGated)
        } else if self.rcc & GPUCFGR_RESET != 0 {
            Some(FaultKind::GpuInReset)
        } else {
            None
        }
    }

    fn gpu_read(&mut self, off: u32) -> u32 {
        match off {
            HI_CLOCK_CONTROL => self.gpu.get(&off).copied().unwrap_or(0) | CLOCK_IDLE_3D | CLOCK_IDLE_2D,
            HI_IDLE_STATE => IDLE_ALL,
            HI_INTR_ACKNOWLEDGE => std::mem::take(&mut self.intr_pending),
            HI_CHIP_IDENTITY => 0,
            HI_CHIP_MODEL => MODEL,
            HI_CHIP_REV => REVISION,
            HI_CHIP_CUSTOMER_ID => 0,
            HI_CHIP_PRODUCT_ID => PRODUCT_ID,
            HI_CHIP_ECO_ID => ECO_ID,
            _ => self.gpu.get(&off).copied().unwrap_or(0),
        }
    }

    fn gpu_write(&mut self, off: u32, val: u32) {
        match off {
            // Read-only: the acknowledge, the idle state and the identity.
            HI_IDLE_STATE | HI_INTR_ACKNOWLEDGE | HI_CHIP_IDENTITY | HI_CHIP_MODEL | HI_CHIP_REV
            | HI_CHIP_CUSTOMER_ID | HI_CHIP_PRODUCT_ID | HI_CHIP_ECO_ID => {}
            // The soft reset is a pulse; the idle bits are read-only.
            HI_CLOCK_CONTROL => {
                self.gpu.insert(off, val & !(CLOCK_SOFT_RESET | CLOCK_IDLE_3D | CLOCK_IDLE_2D));
            }
            _ => {
                self.gpu.insert(off, val);
            }
        }
    }
}

impl Mmio for Mp2Model {
    fn read32(&self, addr: usize) -> u32 {
        let mut s = self.0.borrow_mut();
        if let Some(off) = addr.checked_sub(RCC_BASE).filter(|o| *o < 0x1000) {
            let off = off as u32;
            if off != RCC_GPUCFGR_OFF {
                s.fault(FaultKind::RccOtherRegister, off, false);
                return POISON;
            }
            let v = s.rcc;
            s.log.push(Access::RccR(off, v));
            return v;
        }
        let off = addr.wrapping_sub(GPU_BASE) as u32;
        assert!((GPU_BASE..GPU_BASE + 0x1_0000).contains(&addr), "address {addr:#x} is in no modelled block");
        match s.gpu_fault(off) {
            Some(k) => {
                s.fault(k, off, false);
                POISON
            }
            None => {
                let v = s.gpu_read(off);
                s.log.push(Access::GpuR(off, v));
                v
            }
        }
    }

    fn write32(&self, addr: usize, val: u32) {
        let mut s = self.0.borrow_mut();
        if let Some(off) = addr.checked_sub(RCC_BASE).filter(|o| *o < 0x1000) {
            let off = off as u32;
            s.log.push(Access::RccW(off, val));
            if off != RCC_GPUCFGR_OFF {
                s.fault(FaultKind::RccOtherRegister, off, true);
            } else if !s.domain_on {
                s.fault(FaultKind::RccWhileDomainOff, off, true);
            } else if val & !(GPUCFGR_RESET | GPUCFGR_CLOCK) != 0 {
                s.fault(FaultKind::RccReservedBits, off, true);
            } else {
                s.rcc = val;
            }
            return;
        }
        let off = addr.wrapping_sub(GPU_BASE) as u32;
        assert!((GPU_BASE..GPU_BASE + 0x1_0000).contains(&addr), "address {addr:#x} is in no modelled block");
        s.log.push(Access::GpuW(off, val));
        match s.gpu_fault(off) {
            Some(k) => s.fault(k, off, true),
            None => s.gpu_write(off, val),
        }
    }

    fn read8(&self, _addr: usize) -> u8 {
        panic!("the STM32MP2 GPU path has no byte registers")
    }

    fn write8(&self, _addr: usize, _val: u8) {
        panic!("the STM32MP2 GPU path has no byte registers")
    }
}

impl ScmiServer {
    /// The next command of (`proto`, `msg`) is answered `status` (and does
    /// nothing).
    pub fn fail_next(&self, proto: u8, msg: u8, status: i32) {
        self.0.borrow_mut().script.push((proto, msg, status, Some(1)));
    }

    /// Every command of (`proto`, `msg`) is answered `status`.
    pub fn fail_always(&self, proto: u8, msg: u8, status: i32) {
        self.0.borrow_mut().script.push((proto, msg, status, None));
    }
}

const SCMI_VERSION: u32 = 0x3_0000;
const POWER_ATTR_SYNC: u32 = 1 << 29;
const PERF_ATTR_LEVEL_SET: u32 = 1 << 28;
const PERF_ATTR_LIMIT_SET: u32 = 1 << 29;

fn put(rx: &mut [u8], words: &[u32]) -> usize {
    for (i, w) in words.iter().enumerate() {
        rx[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    4 * words.len()
}

/// A name field: 16 bytes, NUL-padded, as four words.
fn name(n: &str) -> [u32; 4] {
    let mut b = [0u8; 16];
    b[..n.len()].copy_from_slice(n.as_bytes());
    [0, 4, 8, 12].map(|i| scmi::word(&b, i / 4))
}

impl Shared {
    /// The response's words after the header (status first).
    fn serve(&mut self, proto: u8, msg: u8, args: &[u32]) -> Vec<u32> {
        let err = |st: i32| vec![st as u32];
        let ok = |mut v: Vec<u32>| {
            v.insert(0, scmi::STATUS_SUCCESS as u32);
            v
        };
        // A scripted status first.
        if let Some(i) = self.script.iter().position(|e| e.0 == proto && e.1 == msg) {
            let st = self.script[i].2;
            match &mut self.script[i].3 {
                Some(1) => {
                    self.script.remove(i);
                }
                Some(n) => *n -= 1,
                None => {}
            }
            return err(st);
        }
        if proto != scmi::PROTO_POWER && proto != scmi::PROTO_PERF {
            return err(scmi::STATUS_NOT_SUPPORTED);
        }
        let want = |n: usize| args.len() == n;
        match (proto, msg) {
            (_, scmi::PROTOCOL_VERSION) if want(0) => ok(vec![SCMI_VERSION]),
            (scmi::PROTO_POWER, scmi::POWER_STATE_SET) if want(3) => {
                let (flags, dom, state) = (args[0], args[1], args[2]);
                if dom != PD_GPU {
                    return err(scmi::STATUS_NOT_FOUND);
                }
                if flags != 0 {
                    return err(scmi::STATUS_NOT_SUPPORTED);
                }
                if state != scmi::POWER_ON && state != scmi::POWER_OFF {
                    return err(scmi::STATUS_INVALID_PARAMETERS);
                }
                let on = state == scmi::POWER_ON;
                if !on && self.domain_on && self.rcc & GPUCFGR_CLOCK != 0 {
                    self.fault(FaultKind::DomainOffWhileClockOn, 0, true);
                }
                self.domain_on = on;
                self.log.push(Access::PowerSet(dom, state));
                ok(vec![])
            }
            (scmi::PROTO_POWER, scmi::POWER_STATE_GET) if want(1) => {
                if args[0] != PD_GPU {
                    return err(scmi::STATUS_NOT_FOUND);
                }
                ok(vec![if self.domain_on { scmi::POWER_ON } else { scmi::POWER_OFF }])
            }
            (scmi::PROTO_POWER, scmi::POWER_DOMAIN_ATTRIBUTES) if want(1) => {
                if args[0] != PD_GPU {
                    return err(scmi::STATUS_NOT_FOUND);
                }
                let mut v = vec![POWER_ATTR_SYNC];
                v.extend(name("GPU"));
                ok(v)
            }
            (scmi::PROTO_PERF, scmi::PERF_LEVEL_SET) if want(2) => {
                let (dom, level) = (args[0] as usize, args[1]);
                if dom >= PERF_DOMAINS {
                    return err(scmi::STATUS_NOT_FOUND);
                }
                if dom as u32 == PERF_GPU && self.domain_on {
                    return err(scmi::STATUS_DENIED);
                }
                self.levels[dom] = level;
                self.log.push(Access::LevelSet(dom as u32, level));
                ok(vec![])
            }
            (scmi::PROTO_PERF, scmi::PERF_LEVEL_GET) if want(1) => {
                let dom = args[0] as usize;
                if dom >= PERF_DOMAINS {
                    return err(scmi::STATUS_NOT_FOUND);
                }
                ok(vec![self.levels[dom]])
            }
            (scmi::PROTO_PERF, scmi::PERF_DOMAIN_ATTRIBUTES) if want(1) => {
                let dom = args[0] as usize;
                if dom >= PERF_DOMAINS {
                    return err(scmi::STATUS_NOT_FOUND);
                }
                // attributes, rate limit, sustained frequency (kHz),
                // sustained level, name.
                let mut v =
                    vec![PERF_ATTR_LEVEL_SET | PERF_ATTR_LIMIT_SET, 0, self.levels[dom] / 1000, self.levels[dom]];
                v.extend(name(if dom == 0 { "CPU" } else { "GPU" }));
                ok(v)
            }
            (_, scmi::PROTOCOL_VERSION | scmi::POWER_STATE_SET | scmi::POWER_STATE_GET | scmi::PERF_LEVEL_SET)
            | (_, scmi::PERF_LEVEL_GET | scmi::POWER_DOMAIN_ATTRIBUTES) => {
                // A message of this protocol with the wrong payload length.
                if (proto == scmi::PROTO_POWER && matches!(msg, 0x0 | 0x3 | 0x4 | 0x5))
                    || (proto == scmi::PROTO_PERF && matches!(msg, 0x0 | 0x3 | 0x7 | 0x8))
                {
                    err(scmi::STATUS_PROTOCOL_ERROR)
                } else {
                    err(scmi::STATUS_NOT_SUPPORTED)
                }
            }
            _ => err(scmi::STATUS_NOT_SUPPORTED),
        }
    }
}

impl Transport for ScmiServer {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        let mut s = self.0.borrow_mut();
        if tx.len() < 4 || tx.len() % 4 != 0 {
            return Err(-22);
        }
        let hdr = scmi::word(tx, 0);
        let (msg, proto) = ((hdr & 0xff) as u8, ((hdr >> 10) & 0xff) as u8);
        let args: Vec<u32> = (1..tx.len() / 4).map(|i| scmi::word(tx, i)).collect();
        let words = s.serve(proto, msg, &args);
        let mut resp = vec![hdr];
        resp.extend(words);
        let n = put(rx, &resp);
        s.msgs.push(Message { tx: tx.to_vec(), rx: rx[..n].to_vec() });
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::scmi::{Client, Error};
    use crate::gpu::vivante::hw;
    use crate::model::TestEnv;

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn fault_kinds(m: &Mp2Model) -> Vec<FaultKind> {
        m.faults().iter().map(|f| f.kind).collect()
    }

    /// The order the notes give: power domain 0, then the clock gate, then
    /// the reset release (galcore sets no level and no rate).
    fn power_up(m: &Mp2Model, c: &mut Client<ScmiServer>) {
        c.power_set(PD_GPU, scmi::POWER_ON).unwrap();
        let rcc = m.rcc_base() + RCC_GPUCFGR_OFF as usize;
        m.modify32(rcc, 0x2, 0x2);
        m.modify32(rcc, 0x1, 0);
    }

    #[test]
    fn mp2_model_rcc_reset_value_is_gated_and_in_reset() {
        let m = Mp2Model::new();
        assert_eq!(m.rcc(), 1);
        assert!(!m.clock_on() && m.in_reset() && !m.domain_on());
        assert_eq!(m.level(1), PERF_LEVEL_INIT);
        assert_eq!(Mp2Model::with_rcc_reset(0).rcc(), 0);
        assert_eq!(m.read32(RCC_BASE + 0x83c), 1);
        assert!(m.faults().is_empty());
    }

    #[test]
    fn mp2_model_gpu_access_faults_in_order_of_condition() {
        let m = Mp2Model::new();
        let id = GPU_BASE + HI_CHIP_MODEL as usize;
        // Domain off first, whatever the RCC says.
        assert_eq!(m.read32(id), POISON);
        let mut c = Client::new(m.scmi());
        c.power_set(PD_GPU, scmi::POWER_ON).unwrap();
        assert_eq!(m.read32(id), POISON); // clock gated
        m.write32(RCC_BASE + 0x83c, 0x3);
        m.write32(GPU_BASE + HI_INTR_ENBL as usize, 0xffff_ffff); // in reset
        assert_eq!(m.peek(HI_INTR_ENBL), 0);
        m.write32(RCC_BASE + 0x83c, 0x2);
        assert_eq!(m.read32(id), MODEL);
        assert_eq!(fault_kinds(&m), [FaultKind::GpuDomainOff, FaultKind::GpuClockGated, FaultKind::GpuInReset]);
        // The fault follows the access that caused it in the log.
        let a = m.accesses();
        assert_eq!(a[0], Access::Fault(Fault { kind: FaultKind::GpuDomainOff, off: HI_CHIP_MODEL, write: false }));
    }

    #[test]
    fn mp2_model_window_and_rcc_faults() {
        let m = Mp2Model::new();
        // RCC write before the domain is on.
        m.write32(RCC_BASE + 0x83c, 0x2);
        assert_eq!(m.rcc(), 1);
        let mut c = Client::new(m.scmi());
        power_up(&m, &mut c);
        assert!(m.clock_on() && !m.in_reset());
        assert_eq!(m.read32(GPU_BASE + 0x800), POISON);
        assert_eq!(m.read32(GPU_BASE + 0x1000), POISON);
        assert_eq!(m.read32(GPU_BASE + 0x21), POISON);
        m.write32(RCC_BASE + 0x83c, 0x6);
        assert_eq!(m.read32(RCC_BASE + 0x840), POISON);
        // Powering off with the gate open.
        c.power_set(PD_GPU, scmi::POWER_OFF).unwrap();
        assert_eq!(
            fault_kinds(&m),
            [
                FaultKind::RccWhileDomainOff,
                FaultKind::GpuOutsideWindow,
                FaultKind::GpuOutsideWindow,
                FaultKind::GpuOutsideWindow,
                FaultKind::RccReservedBits,
                FaultKind::RccOtherRegister,
                FaultKind::DomainOffWhileClockOn,
            ]
        );
    }

    #[test]
    fn mp2_model_gpu_registers() {
        let m = Mp2Model::new();
        let mut c = Client::new(m.scmi());
        power_up(&m, &mut c);
        let r = |o: u32| m.read32(GPU_BASE + o as usize);
        assert_eq!(r(HI_CHIP_MODEL), 0x8000);
        assert_eq!(r(HI_CHIP_REV), 0x6205);
        assert_eq!(r(HI_CHIP_PRODUCT_ID), 0x80003);
        assert_eq!(r(HI_CHIP_ECO_ID), 0);
        assert_eq!(r(HI_CHIP_CUSTOMER_ID), 0);
        assert_eq!(r(HI_IDLE_STATE) & IDLE_ALL, IDLE_ALL);
        assert_eq!(r(HI_CLOCK_CONTROL) & (CLOCK_IDLE_3D | CLOCK_IDLE_2D), CLOCK_IDLE_3D | CLOCK_IDLE_2D);
        // HI_INTR_ENBL stores; the identity does not.
        m.write32(GPU_BASE + HI_INTR_ENBL as usize, 0x8000_0001);
        assert_eq!(r(HI_INTR_ENBL), 0x8000_0001);
        m.write32(GPU_BASE + HI_CHIP_MODEL as usize, 0);
        assert_eq!(r(HI_CHIP_MODEL), 0x8000);
        // The acknowledge reads the pending vector once.
        m.raise_interrupt(4);
        assert_eq!(r(HI_INTR_ACKNOWLEDGE), 4);
        assert_eq!(r(HI_INTR_ACKNOWLEDGE), 0);
        assert!(m.faults().is_empty());
    }

    #[test]
    fn mp2_model_scmi_round_trip_bytes() {
        let m = Mp2Model::new();
        let mut c = Client::new(m.scmi());
        assert_eq!(c.version(scmi::PROTO_POWER).unwrap(), 0x3_0000);
        assert_eq!(c.version(scmi::PROTO_PERF).unwrap(), 0x3_0000);
        assert_eq!(c.perf_level_get(PERF_GPU).unwrap(), PERF_LEVEL_INIT);
        c.perf_level_set(PERF_GPU, 700_000).unwrap();
        assert_eq!(c.perf_level_get(PERF_GPU).unwrap(), 700_000);
        assert_eq!(c.power_get(PD_GPU).unwrap(), scmi::POWER_OFF);
        c.power_set(PD_GPU, scmi::POWER_ON).unwrap();
        assert_eq!(c.power_get(PD_GPU).unwrap(), scmi::POWER_ON);
        let a = c.call(scmi::PROTO_POWER, scmi::POWER_DOMAIN_ATTRIBUTES, &[PD_GPU], 5).unwrap();
        assert_eq!((a.get(0), a.get(1)), (POWER_ATTR_SYNC, u32::from_le_bytes(*b"GPU\0")));
        let p = c.call(scmi::PROTO_PERF, scmi::PERF_DOMAIN_ATTRIBUTES, &[PERF_GPU], 8).unwrap();
        assert_eq!(p.get(3), 700_000);
        assert_eq!(p.get(4), u32::from_le_bytes(*b"GPU\0"));

        // Header: message id 7:0, protocol 17:10, token 27:18; the token
        // counts up from 0 per command.
        let h = |proto: u8, msg: u8, tok: u32| (msg as u32) | (proto as u32) << 10 | tok << 18;
        let msgs = m.messages();
        let want: [(Vec<u8>, Vec<u8>); 7] = [
            (le(&[h(0x11, 0x0, 0)]), le(&[h(0x11, 0x0, 0), 0, 0x3_0000])),
            (le(&[h(0x13, 0x0, 1)]), le(&[h(0x13, 0x0, 1), 0, 0x3_0000])),
            (le(&[h(0x13, 0x8, 2), 1]), le(&[h(0x13, 0x8, 2), 0, 800_000])),
            (le(&[h(0x13, 0x7, 3), 1, 700_000]), le(&[h(0x13, 0x7, 3), 0])),
            (le(&[h(0x13, 0x8, 4), 1]), le(&[h(0x13, 0x8, 4), 0, 700_000])),
            (le(&[h(0x11, 0x5, 5), 0]), le(&[h(0x11, 0x5, 5), 0, 1 << 30])),
            (le(&[h(0x11, 0x4, 6), 0, 0, 0]), le(&[h(0x11, 0x4, 6), 0])),
        ];
        for (i, (tx, rx)) in want.iter().enumerate() {
            assert_eq!((&msgs[i].tx, &msgs[i].rx), (tx, rx), "message {i}");
        }
        assert_eq!(msgs.len(), 10);
        assert!(m.faults().is_empty());
    }

    #[test]
    fn mp2_model_scmi_statuses() {
        let m = Mp2Model::new();
        let mut c = Client::new(m.scmi());
        assert_eq!(c.power_get(7), Err(Error::Status(scmi::STATUS_NOT_FOUND)));
        assert_eq!(c.perf_level_get(2), Err(Error::Status(scmi::STATUS_NOT_FOUND)));
        assert_eq!(c.version(scmi::PROTO_CLOCK), Err(Error::Status(scmi::STATUS_NOT_SUPPORTED)));
        assert_eq!(
            c.call(scmi::PROTO_POWER, scmi::POWER_STATE_SET, &[1, PD_GPU, 0], 0).err(),
            Some(Error::Status(scmi::STATUS_NOT_SUPPORTED))
        );
        assert_eq!(
            c.call(scmi::PROTO_POWER, scmi::POWER_STATE_SET, &[0, PD_GPU, 5], 0).err(),
            Some(Error::Status(scmi::STATUS_INVALID_PARAMETERS))
        );
        assert_eq!(
            c.call(scmi::PROTO_POWER, scmi::POWER_STATE_GET, &[], 0).err(),
            Some(Error::Status(scmi::STATUS_PROTOCOL_ERROR))
        );
        // Scripted: one DENIED, then the real answer; NOT_FOUND for good.
        m.scmi().fail_next(scmi::PROTO_POWER, scmi::POWER_STATE_SET, scmi::STATUS_DENIED);
        assert_eq!(c.power_set(PD_GPU, scmi::POWER_ON), Err(Error::Status(scmi::STATUS_DENIED)));
        assert!(!m.domain_on());
        c.power_set(PD_GPU, scmi::POWER_ON).unwrap();
        m.scmi().fail_always(scmi::PROTO_PERF, scmi::PERF_LEVEL_GET, scmi::STATUS_NOT_FOUND);
        for _ in 0..2 {
            assert_eq!(c.perf_level_get(PERF_GPU), Err(Error::Status(scmi::STATUS_NOT_FOUND)));
        }
        // A level change while the GPU is on is refused by the real server.
        assert_eq!(c.perf_level_set(PERF_GPU, 1), Err(Error::Status(scmi::STATUS_DENIED)));
        assert_eq!(m.level(1), PERF_LEVEL_INIT);
        // The CPU's domain is not the GPU's.
        c.perf_level_set(0, 5).unwrap();
        assert_eq!(m.level(0), 5);
        assert!(m.faults().is_empty());
    }

    #[test]
    fn mp2_model_power_up_order_ends_clean_with_identity() {
        let m = Mp2Model::new();
        let mut c = Client::new(m.scmi());
        // The level is read, never set (galcore sets none), then the domain.
        let level = c.perf_level_get(PERF_GPU).unwrap();
        power_up(&m, &mut c);
        let e = TestEnv::default();
        let id = hw::identify(&m, m.gpu_base(), &e).unwrap();
        assert_eq!((id.model, id.revision), (0x8000, 0x6205));
        assert_eq!((id.product_id, id.eco_id), (0x80003, 0));
        hw::reset(&m, m.gpu_base(), &e, &id).unwrap();
        m.write32(GPU_BASE + HI_INTR_ENBL as usize, 0x8000_0003);
        assert_eq!(m.read32(GPU_BASE + HI_INTR_ENBL as usize), 0x8000_0003);
        // Teardown in the reverse order.
        m.modify32(RCC_BASE + 0x83c, 0x3, 0x1);
        c.power_set(PD_GPU, scmi::POWER_OFF).unwrap();
        assert_eq!(m.faults(), []);
        assert_eq!(m.level(1), level);
        assert!(m.accesses().iter().all(|a| !matches!(a, Access::LevelSet(..))));
        // Nothing was read at or above the window.
        assert!(m.accesses().iter().all(|a| match a {
            Access::GpuR(o, _) | Access::GpuW(o, _) => *o < GPU_WINDOW,
            _ => true,
        }));
    }

    #[test]
    fn mp2_model_wrong_order_is_caught() {
        // etnaviv-like: the clock gate before the domain, then the GPU.
        let m = Mp2Model::new();
        m.write32(RCC_BASE + 0x83c, 0x2);
        let _ = m.read32(GPU_BASE + HI_CHIP_MODEL as usize);
        assert_eq!(fault_kinds(&m), [FaultKind::RccWhileDomainOff, FaultKind::GpuDomainOff]);
    }
}
