// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the paths the Zephyr samples `gc_fence` and `gc_clear`
//! drive through `ffi::vivante` (identify, reset, hw init, the MMU, the
//! ring, a submit with its event, the interrupt), on the model bus with the
//! STM32MP257F's identity registers (model 0x8000, revision 0x6205, product
//! 0x80003, eco 0; issue #1281) instead of the i.MX8MP's. The carveout runs
//! at the i.MX8MP's base and at the DK's `gpu-reserved` area (0xfa800000).
//!
//! The model holds registers only; it does not execute a command stream.
//! [`FrontEnd`] is this file's stand-in for the GPU: from the ring's first
//! word it follows the LINKs the driver wrote, applies the LOAD_STATEs the
//! resolve engine needs, fills the window on the RS kick and raises the
//! EVENT's bit in INTR_ACKNOWLEDGE. A wrong LINK or event word shows as a
//! stream that never reaches the kick, the event or the idle loop.

use super::regs::*;
use super::*;
use crate::errno::*;
use crate::model::{Model, TestEnv, GC};

const CLOCK_HZ: u32 = 800_000_000;
const CARVEOUT: usize = 8 * 1024 * 1024;
/// Physical bases: the i.MX8MP's carveout and the DK's `gpu-reserved` area.
const BASE_IMX8MP: u64 = 0x5800_0000;
const BASE_STM32MP257F: u64 = 0xfa80_0000;

// The resolve engine's registers (rnndb state_3d.xml.h), repeated here on
// purpose so a wrong constant in the driver cannot agree with itself.
const SYNC_RECIPIENT_RA: u32 = 5;
const RS_KICKER: u32 = 0x1600;
const RS_DEST_STRIDE: u32 = 0x1614;
const RS_WINDOW_SIZE: u32 = 0x1620;
const RS_CLEAR_CONTROL: u32 = 0x163c;
const RS_FILL_VALUE: u32 = 0x1640;
const RS_PIPE_DEST_ADDR: u32 = 0x16e0;

/// A zeroed, 8-byte aligned carveout.
struct Carveout(Vec<u64>);

/// The ST row's identity registers on top of the i.MX8MP's scenario (the
/// feature words and the build date are not part of the row).
fn poke_st_identity(m: &Model) {
    for (off, v) in [
        (0x18, 0x8000_6205),
        (0x20, 0x8000),
        (0x24, 0x6205),
        (0x28, 0x2020_0129),
        (0x2c, 0x0000_1234),
        (0xa8, 0x80003),
        (0xe8, 0),
        (0x1c, 0xe068_7ca5),
        (0x34, 0xc979_9eff),
    ] {
        m.poke(GC, off, v);
    }
}

/// The GPU of a bring-up the way `gc_render_init` leaves it: identified,
/// reset, initialised, MMU on, ring running.
struct Bringup {
    m: Model,
    e: TestEnv,
    id: Box<GcIdentity>,
    _carveout: Carveout,
    c: GcCore,
}

fn bring_up(phys: u64) -> Bringup {
    let m = Model::new();
    let e = TestEnv::default();
    poke_st_identity(&m);
    let regs = m.base(GC);
    let id = Box::new(hw::identify(&m, regs, &e).unwrap());
    hw::reset(&m, regs, &e, &id).unwrap();
    hw::init(&m, regs, &e);
    let mut carveout = Carveout(vec![0u64; CARVEOUT / 8]);
    let mut c = GcCore::new(regs, carveout.0.as_mut_ptr().cast(), phys, CARVEOUT);
    c.id = &*id;
    mmu::init(&mut c, &e).unwrap();
    mmu::enable(&mut c, &m, &e).unwrap();
    cmdbuf::ring_init(&mut c, &m, &e, CLOCK_HZ);
    c.event_free = EVENT_ALL;
    Bringup { m, e, id, _carveout: carveout, c }
}

/// What the stream did when the front end walked it.
#[derive(Default, Debug)]
struct Walk {
    /// Event ids raised, in order.
    events: Vec<u32>,
    /// RS kicks executed.
    kicks: u32,
    /// The front end reached an idle loop (a WAIT) rather than an END or a
    /// word it does not know.
    idle: bool,
}

/// The test's GPU: states by offset, the resolve engine's clear.
struct FrontEnd<'a> {
    c: &'a GcCore,
    states: std::collections::BTreeMap<u32, u32>,
}

impl FrontEnd<'_> {
    fn word(&self, gpu: u32) -> u32 {
        let off = self
            .c
            .carveout_offset(gpu)
            .unwrap_or_else(|| panic!("front end fetch at 0x{gpu:08x} outside the carveout"));
        self.c.mem().r32(off)
    }

    fn st(&self, reg: u32) -> u32 {
        self.states.get(&reg).copied().unwrap_or(0)
    }

    /// Follow the stream from the ring's first word.
    fn run(&mut self) -> Walk {
        let mut w = Walk::default();
        let mut pc = self.c.ring_gpu;
        for _ in 0..100_000 {
            let hdr = self.word(pc);
            match hdr & 0xf800_0000 {
                VIV_FE_LOAD_STATE_HEADER_OP_LOAD_STATE => {
                    let count = (hdr & VIV_FE_LOAD_STATE_HEADER_COUNT__MASK) >> VIV_FE_LOAD_STATE_HEADER_COUNT__SHIFT;
                    let reg = (hdr & VIV_FE_LOAD_STATE_HEADER_OFFSET__MASK) << VIV_FE_LOAD_STATE_HEADER_OFFSET__SHR;
                    for i in 0..count {
                        let v = self.word(pc + 4 * (1 + i));
                        self.state(reg + 4 * i, v, &mut w);
                    }
                    pc += 4 * ((1 + count + 1) & !1);
                }
                VIV_FE_NOP_HEADER_OP_NOP | VIV_FE_STALL_HEADER_OP_STALL => pc += 8,
                VIV_FE_LINK_HEADER_OP_LINK => pc = self.word(pc + 4),
                VIV_FE_WAIT_HEADER_OP_WAIT => {
                    w.idle = true;
                    return w;
                }
                _ => return w,
            }
        }
        panic!("the front end did not settle in 100000 commands");
    }

    fn state(&mut self, reg: u32, v: u32, w: &mut Walk) {
        if reg == VIVS_GL_EVENT {
            w.events.push(v & VIVS_GL_EVENT_EVENT_ID__MASK);
        }
        self.states.insert(reg, v);
        if reg == RS_KICKER {
            w.kicks += 1;
            self.resolve_clear();
        }
    }

    /// The resolve engine with a fill value: `RS_WINDOW_SIZE` pixels of
    /// `RS_DEST_STRIDE` bytes a row, from `RS_PIPE_DEST_ADDR(0)`.
    fn resolve_clear(&self) {
        assert_eq!(self.st(RS_CLEAR_CONTROL) >> 16, 0xffff, "clear mask");
        let (width, height) = (self.st(RS_WINDOW_SIZE) & 0xffff, self.st(RS_WINDOW_SIZE) >> 16);
        assert_eq!(width % 16, 0, "the resolve engine aborts on a width that is not a multiple of 16");
        let (stride, dest) = (self.st(RS_DEST_STRIDE), self.st(RS_PIPE_DEST_ADDR));
        let mem = self.c.mem();
        let base = self.c.carveout_offset(dest).expect("resolve destination outside the carveout");
        for y in 0..height as usize {
            for x in 0..width as usize {
                mem.w32(base + y * stride as usize + x * 4, self.st(RS_FILL_VALUE));
            }
        }
    }
}

impl Bringup {
    /// Claim an event, submit the buffer at `cmd_gpu`, let the GPU run the
    /// ring and take its interrupt.
    fn submit_and_retire(&mut self, cmd_gpu: u32, cmd_bytes: u32, fence: u32) -> (Walk, irq::Handled) {
        let event = irq::claim(&mut self.c, fence).unwrap().expect("a free event");
        let s = GcSubmit { cmd_gpu, cmd_bytes, flags: submit::FLUSH_COLOR | submit::FLUSH_DEPTH | submit::FLUSH_TEX };
        cmdbuf::ring_submit(&mut self.c, &self.e, &s, event, fence, CLOCK_HZ).unwrap();
        let walk = FrontEnd { c: &self.c, states: Default::default() }.run();
        for &ev in &walk.events {
            // the event reaches INTR_ACKNOWLEDGE as its bit
            self.m.poke(GC, VIVS_HI_INTR_ACKNOWLEDGE, self.m.peek(GC, VIVS_HI_INTR_ACKNOWLEDGE) | 1 << ev);
        }
        let h = irq::handle(&mut self.c, &self.m, 1000);
        (walk, h)
    }

    /// Carveout offset and GPU address of `byte` into the pool.
    fn pool(&self, byte: usize) -> (usize, u32) {
        (OFF_POOL + byte, (self.c.carveout_phys + (OFF_POOL + byte) as u64) as u32)
    }
}

#[test]
fn identify_gives_the_st_row_with_16_streams() {
    for phys in [BASE_IMX8MP, BASE_STM32MP257F] {
        let b = bring_up(phys);
        let id = &*b.id;
        assert!(id.from_table, "the ST registers are in the table");
        assert_eq!((id.model, id.revision, id.product_id, id.eco_id), (0x8000, 0x6205, 0x80003, 0));
        assert_eq!(id.stream_count, 16, "streams");
        assert_eq!(id.stream_count, hw::DB_STM32MP257F.streams);
        assert!(id.secure_ahb);
        // hw init wrote what it reads back: every event and both error bits
        assert_eq!(b.m.peek(GC, VIVS_HI_INTR_ENBL), 0xffff_ffff);
        assert!(b.c.mmu_on && b.c.fe_running);
    }
}

#[test]
fn reset_and_init_say_the_gc_diag_lines_on_the_st_identity() {
    let b = bring_up(BASE_STM32MP257F);
    let lines = b.e.lines.borrow();
    let d: Vec<&str> = lines.iter().filter(|l| l.1.starts_with("gc diag: ")).map(|l| l.1.as_str()).collect();
    assert_eq!(d.len(), 3, "{d:?}");
    assert!(d[0].starts_with("gc diag: reset-attempt n=1 004=0x"), "{}", d[0]);
    assert!(d[1].starts_with("gc diag: after-reset 3a8=0x"), "{}", d[1]);
    assert!(d[2].starts_with("gc diag: after-init 3a8=0x"), "{}", d[2]);
}

#[test]
fn a_fence_retires_with_its_interrupt() {
    for phys in [BASE_IMX8MP, BASE_STM32MP257F] {
        let mut b = bring_up(phys);
        // gc_fence: 64 bytes of NOPs
        let (off, gpu) = b.pool(0);
        let mem = b.c.mem();
        for i in 0..16 {
            mem.w32(off + i * 4, VIV_FE_NOP_HEADER_OP_NOP);
        }
        let (walk, h) = b.submit_and_retire(gpu, 64, 1);
        assert!(walk.idle, "the stream ran back to the idle loop: {walk:?}");
        assert_eq!(walk.events.len(), 1);
        assert!(h.ours && h.freed == 1, "{h:?}");
        assert_eq!(b.c.fence_done(), 1, "base {phys:#x}");
        assert_eq!(b.c.event_free, EVENT_ALL, "the event is free again");
        assert_eq!(b.c.irq_errors, 0);
        // the next fence rides the re-armed ring
        let (walk, h) = b.submit_and_retire(gpu, 64, 2);
        assert!(walk.idle && h.ours);
        assert_eq!(b.c.fence_done(), 2);
    }
}

const WIDTH: u32 = 256;
const HEIGHT: u32 = 128;
const POISON: u32 = 0x00a5_5a3c;

/// `gc_draw_clear`'s stream (gc_draw.c, the states the resolve engine
/// needs), built with the kernel side's own encoder.
fn build_clear(b: &Bringup, argb: u32, target: u32) -> (u32, u32) {
    let (off, gpu) = b.pool(0);
    let mem = b.c.mem();
    let mut c = cmd::Cmd::new(&mem, off, gpu, 128);
    c.state(VIVS_GL_FLUSH_CACHE, VIVS_GL_FLUSH_CACHE_COLOR | VIVS_GL_FLUSH_CACHE_DEPTH);
    c.stall(SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);
    c.state(RS_DEST_STRIDE, WIDTH * 4);
    c.state(RS_PIPE_DEST_ADDR, target);
    c.state(RS_WINDOW_SIZE, HEIGHT << 16 | WIDTH);
    c.state(RS_CLEAR_CONTROL, 0xffff << 16 | 1);
    c.state_multi(RS_FILL_VALUE, &[argb; 4]);
    c.state(RS_KICKER, 0xbeeb_beeb);
    c.stall(SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);
    c.align8();
    (gpu, c.n * 4)
}

#[test]
fn a_clear_reads_back_through_the_resolve_engine_with_0_wrong_pixels() {
    for phys in [BASE_IMX8MP, BASE_STM32MP257F] {
        let mut b = bring_up(phys);
        let (t_off, t_gpu) = b.pool(0x10000);
        let mem = b.c.mem();
        let guard = 2 * WIDTH as usize; // two guard rows after the target
        let pixels = (WIDTH * HEIGHT) as usize;
        for i in 0..pixels + guard {
            mem.w32(t_off + i * 4, POISON);
        }
        for (n, colour) in [(1u32, 0xff12_3456u32), (2, 0x0065_4321)] {
            let (gpu, bytes) = build_clear(&b, colour, t_gpu);
            let (walk, h) = b.submit_and_retire(gpu, bytes, n);
            assert_eq!(walk.kicks, 1, "the stream reaches the resolve engine's kick");
            assert!(walk.idle && h.ours);
            assert_eq!(b.c.fence_done(), n);
            let wrong = (0..pixels).filter(|&i| mem.r32(t_off + i * 4) != colour).count();
            let wrong_guard = (pixels..pixels + guard).filter(|&i| mem.r32(t_off + i * 4) != POISON).count();
            assert_eq!((wrong, wrong_guard), (0, 0), "clear {n} at base {phys:#x}");
        }
    }
}

#[test]
fn a_submit_outside_the_carveout_is_refused_on_the_st_identity() {
    let mut b = bring_up(BASE_STM32MP257F);
    let s = GcSubmit { cmd_gpu: BASE_IMX8MP as u32, cmd_bytes: 64, flags: 0 };
    assert_eq!(cmdbuf::ring_submit(&mut b.c, &b.e, &s, 0, 1, CLOCK_HZ), Err(-EFAULT));
}
