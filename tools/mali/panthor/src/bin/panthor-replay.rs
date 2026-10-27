// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! panthor-replay: run the jobs of a Mesa pandecode trace again through this
//! crate, from the raw buffer bytes the trace's memory dumps hold, and count
//! the pixels of the last linear render target.
//!
//! Input: a trace taken with `PAN_MESA_DEBUG=dump,trace,sync`, whose file
//! holds the CS words pandecode interpreted and, after each submit, a dump
//! of every buffer as the GPU left it. The board's triangle trace has two
//! dumps: after the frame (clear blue, red triangle into a 64x64 AFBC
//! target) and after the read-back blit (the AFBC target sampled into a
//! 64x64 linear RGBA8 target).
//!
//! What is replayed and what is changed, each from a cited source:
//! - Every dumped buffer is bound at the GPU VA the trace used, so the
//!   addresses inside descriptors and streams stay valid without editing
//!   them. A bind the kernel refuses ends the run; no other VA is tried.
//!   Buffers that hold a shader the trace printed are bound executable and
//!   read-only, the rest no-exec (Mesa's rule, panthor_kmod.c:1163-1169).
//! - The kernel's objects are this process's own: the tiler heap context
//!   and first chunk come from our TILER_HEAP_CREATE. The context init
//!   stream (the words before the first dumped stream) gets our heap
//!   context in its `MOVE48 d72` (Mesa's `HEAP_SET` source,
//!   pan_csf.c:1917-1919), and the Tiler Heap descriptor gets our first
//!   chunk with Mesa's initial bottom and top (base + 64, base + size;
//!   pan_csf.c:1880-1885).
//! - A dump is the state after a job, so what the GPU writes is put back to
//!   what Mesa wrote before it: the render targets are zeroed (also the
//!   poison that shows a target the replay did not write), and the
//!   tiler context's GPU-written fields (Polygon List, Completed top and
//!   bottom, Private state; v10.xml:2008-2023) are zeroed, as Mesa's
//!   `pan_pack` leaves them (pan_csf.c:970-1000). Before the second job the
//!   buffers its stream names that changed between the dumps (its stream
//!   and descriptor pool) are loaded from the second dump.
//! - Each dumped stream is the dumped buffer whose words, from offset 0,
//!   hold the trace's top-level words in order; its size ends at the last.
//!
//! The verdict compares the replay's pixel counts with the same count over
//! the trace's own dump of that target (the counter's known-good case), and
//! the target's bytes with the dump's; the frame's AFBC target is compared
//! byte for byte with the first dump and the difference reported.
//!
//! Usage: panthor-replay [--skip-frame] [--shift-heap] <trace with dumps> [/dev/dri/renderD128]
//!
//! `--skip-frame` is the control: the frame job is not submitted, so the
//! blit samples the zeroed AFBC target, and the count must hold no red and
//! no blue (it shows that the counted pixels come from the replayed frame).
//!
//! `--shift-heap` exercises the heap patches: the kernel normally returns
//! the VAs the trace had, so patching the init stream's `MOVE48 d72` and the
//! heap descriptor changes nothing. With this flag a spare tiler heap is made
//! (and kept) before the real one, so the real one lands elsewhere; the run
//! refuses unless both the context VA and the first chunk VA differ from the
//! trace's, and checks that the patched descriptor is the one job 1's tiler
//! context points at.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::atomic::{fence, Ordering};
use std::time::Duration;
use yakogl_mali_panthor::cs::{self, OP_MOVE48};
use yakogl_mali_panthor::dev::{Bo, Device, Error, Group, Map, Result, TilerHeap, Vm, Waited};
use yakogl_mali_panthor::pandecode::{count_rgba8, CsLine, PixelCount, Trace};

const TIMEOUT: Duration = Duration::from_secs(5);
/// `RUN_IDVS` (v10.xml:518-558) and `HEAP_SET`.
const OP_RUN_IDVS: u64 = 6;
const OP_HEAP_SET: u64 = 48;
/// `IDVS SR` TILER_CTX (v10.xml:914) and Mesa's heap context register
/// (pan_csf.c:1917).
const SR_TILER_CTX: u64 = 40;
const REG_HEAP_CTX: u64 = 72;

fn refuse<T>(s: String) -> Result<T> {
    Err(Error::Refused(s))
}

/// The bytes a dump holds for the buffer at `va`, or a refusal naming it.
fn dump_at<'a>(dump: &'a BTreeMap<u64, Vec<u8>>, which: &str, va: u64) -> Result<&'a [u8]> {
    dump.get(&va)
        .map(Vec::as_slice)
        .ok_or_else(|| Error::Refused(format!("the {which} dump holds no buffer at {va:#x}")))
}

/// Where a submitted stream sits: its buffer and its size.
struct StreamAt {
    va: u64,
    size: u32,
    /// Top-level words before it that no dumped buffer holds (a previous
    /// submit: the context init).
    before: Vec<u64>,
}

/// Find the dumped buffer whose words from offset 0 hold `words[j..]` in
/// order, for the smallest `j`.
fn locate(words: &[CsLine], dump: &BTreeMap<u64, Vec<u8>>) -> Option<StreamAt> {
    for j in 0..words.len() {
        for (&va, bytes) in dump {
            let buf: Vec<u64> = bytes.as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
            if buf.first() != Some(&words[j].word) {
                continue;
            }
            let mut pos = 0;
            let mut last = 0;
            let mut ok = true;
            for w in &words[j..] {
                match buf[pos..].iter().position(|b| *b == w.word) {
                    Some(p) => {
                        last = pos + p;
                        pos = last + 1;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return Some(StreamAt {
                    va,
                    size: ((last + 1) * 8) as u32,
                    before: words[..j].iter().map(|w| w.word).collect(),
                });
            }
        }
    }
    None
}

fn move48_imm(w: u64) -> Option<(u64, u64)> {
    (w >> 56 == OP_MOVE48).then_some(((w >> 48) & 0xff, w & 0xffff_ffff_ffff))
}

/// The tiler context a stream's first draw names (`MOVE48 d40` before the
/// first `RUN_IDVS`).
fn tiler_ctx(words: &[CsLine]) -> Option<u64> {
    let idvs = words.iter().position(|w| w.word >> 56 == OP_RUN_IDVS)?;
    words[..idvs].iter().rev().find_map(|w| move48_imm(w.word).filter(|(r, _)| *r == SR_TILER_CTX).map(|(_, v)| v))
}

struct Mem<'v> {
    bos: BTreeMap<u64, Bo<'v>>,
}

impl<'v> Mem<'v> {
    fn bo_of(&mut self, va: u64) -> Result<(&mut Bo<'v>, usize)> {
        let (_, bo) = self
            .bos
            .range_mut(..=va)
            .next_back()
            .filter(|(&base, bo)| va < base + bo.size)
            .ok_or_else(|| Error::Refused(format!("{va:#x} is in no buffer of the trace")))?;
        let off = (va - bo.va()) as usize;
        Ok((bo, off))
    }

    fn zero(&mut self, va: u64, len: usize) -> Result<()> {
        let (bo, off) = self.bo_of(va)?;
        bo.write(off, &vec![0u8; len])
    }

    fn u64_at(&mut self, va: u64) -> Result<u64> {
        let (bo, off) = self.bo_of(va)?;
        bo.read_u64(off)
    }

    fn set_u64(&mut self, va: u64, v: u64) -> Result<()> {
        let (bo, off) = self.bo_of(va)?;
        bo.write_u64(off, v)
    }

    fn load(&mut self, va: u64, bytes: &[u8]) -> Result<()> {
        let bo = self.bos.get_mut(&va).ok_or_else(|| Error::Refused(format!("no buffer at {va:#x}")))?;
        bo.write(0, bytes)
    }
}

/// Zero the GPU-written fields of a Tiler Context (v10.xml:2008-2023):
/// Polygon List (bytes 0-7), Completed top and bottom (40-55), Private
/// state (96-127).
fn reset_tiler_ctx(mem: &mut Mem<'_>, va: u64) -> Result<()> {
    mem.zero(va, 8)?;
    mem.zero(va + 40, 16)?;
    mem.zero(va + 96, 32)
}

fn wait_job(group: &Group<'_>, vm: &Vm<'_>, name: &str, f: &yakogl_mali_panthor::dev::Fence<'_>) -> Result<()> {
    let (w, st) = group.wait(f, TIMEOUT)?;
    match w {
        Waited::Signalled(d) => println!(
            "{name}: fence signalled, wait {} us, submit to wake {} us; group state {:#x}; vm usable {}",
            d.as_micros(),
            f.submitted.elapsed().as_micros(),
            st.state,
            vm.usable()?
        ),
        Waited::TimedOut => return refuse(format!("{name}: fence not signalled within {} s", TIMEOUT.as_secs())),
    }
    Ok(())
}

fn run(trace_path: &str, dev_path: &str, skip_frame: bool, shift_heap: bool) -> Result<bool> {
    let text =
        std::fs::read_to_string(trace_path).map_err(|err| Error::Os { what: format!("read {trace_path}"), err })?;
    let trace = Trace::parse(&text).map_err(Error::Refused)?;
    let jobs: Vec<_> = trace.segments.iter().filter(|s| !s.dump.is_empty()).collect();
    println!(
        "trace {trace_path}: {} segments, {} with a dump, {} render targets, {} shaders",
        trace.segments.len(),
        jobs.len(),
        trace.render_targets.len(),
        trace.shaders.len()
    );
    if jobs.len() != 2 {
        return refuse(format!("{} dumps: this replay knows the frame + read-back pair only", jobs.len()));
    }
    // Where each job's stream is.
    let mut at = Vec::new();
    for (k, seg) in jobs.iter().enumerate() {
        let s = locate(&seg.cs, &seg.dump).ok_or_else(|| {
            Error::Refused(format!("job {k}: no dumped buffer holds its {} stream words", seg.cs.len()))
        })?;
        println!(
            "job {k}: stream at {:#x}, {} bytes ({} words), {} top-level words before it, dump at line {}",
            s.va,
            s.size,
            s.size / 8,
            s.before.len(),
            seg.dump_line
        );
        if k > 0 && !s.before.is_empty() {
            return refuse(format!("job {k}: {} stream words no buffer explains", s.before.len()));
        }
        at.push(s);
    }
    let init = at[0].before.clone();
    if !init.iter().any(|w| w >> 56 == OP_HEAP_SET) {
        return refuse("the words before job 0 hold no HEAP_SET: not a context init stream".into());
    }

    let dev = Device::open(dev_path)?;
    println!(
        "device {dev_path}: {} {}.{}.{}, gpu_id {:#010x}",
        dev.driver.name, dev.driver.major, dev.driver.minor, dev.driver.patch, dev.gpu.gpu_id
    );
    let vm = Vm::create(&dev)?;
    let group = Group::new(&vm, 64 * 1024)?;
    // The spare heap is kept to the end of the run so the real one cannot
    // get its VAs back.
    let spare = if shift_heap { Some(TilerHeap::new(&vm, 2 << 20, 5, 64, 65535)?) } else { None };
    if let Some(sp) = &spare {
        println!(
            "spare tiler heap: ctx {:#x} first chunk {:#x} (kept so the real one lands elsewhere)",
            sp.ctx_va, sp.first_chunk_va
        );
    }
    let heap = TilerHeap::new(&vm, 2 << 20, 5, 64, 65535)?;
    println!(
        "vm {} (user VA {:#x}), group {}, tiler heap ctx {:#x} first chunk {:#x}",
        vm.id, vm.user_va_range, group.handle, heap.ctx_va, heap.first_chunk_va
    );

    // Every buffer of either dump, at its VA.
    let mut sizes: BTreeMap<u64, usize> = BTreeMap::new();
    for seg in &jobs {
        for (&va, b) in &seg.dump {
            let e = sizes.entry(va).or_insert(b.len());
            if *e != b.len() {
                return refuse(format!("buffer {va:#x}: {} bytes in one dump, {} in another", e, b.len()));
            }
        }
    }
    let mut mem = Mem { bos: BTreeMap::new() };
    for (&va, &size) in &sizes {
        let code = trace.shaders.iter().any(|&(s, _)| s >= va && s < va + size as u64);
        let bo = Bo::new_at(&vm, &format!("trace {va:#x}"), size as u64, if code { Map::Code } else { Map::Data }, va)?;
        if bo.size != size as u64 {
            return refuse(format!("buffer {va:#x}: the kernel allocated {:#x} for {size:#x}", bo.size));
        }
        mem.bos.insert(va, bo);
    }
    println!(
        "bound {} trace buffers ({} executable)",
        mem.bos.len(),
        sizes.iter().filter(|(&va, &sz)| trace.shaders.iter().any(|&(s, _)| s >= va && s < va + sz as u64)).count()
    );
    for (va, b) in &jobs[0].dump {
        mem.load(*va, b)?;
    }

    // Render targets: zeroed whole (their buffers), also as the poison.
    let mut rt_bufs = Vec::new();
    for rt in &trace.render_targets {
        let base = mem.bo_of(rt.base)?.0.va();
        if !rt_bufs.contains(&base) {
            rt_bufs.push(base);
        }
    }
    for &b in &rt_bufs {
        let size =
            mem.bos.get(&b).ok_or_else(|| Error::Refused(format!("render target buffer {b:#x} is not bound")))?.size
                as usize;
        mem.zero(b, size)?;
    }
    println!("render targets: {} (buffers {:x?}), zeroed", trace.render_targets.len(), rt_bufs);

    // Job 0's tiler context and the heap descriptor it names.
    let tctx0 = tiler_ctx(&jobs[0].cs).ok_or_else(|| Error::Refused("job 0: no MOVE48 d40 before RUN_IDVS".into()))?;
    reset_tiler_ctx(&mut mem, tctx0)?;
    let heap_desc = mem.u64_at(tctx0 + 24)?;
    let word0 = mem.u64_at(heap_desc)?;
    let size = word0 >> 32;
    if size != heap.chunk_size as u64 {
        return refuse(format!(
            "tiler heap descriptor {heap_desc:#x}: size {size:#x}, our chunks are {:#x}",
            heap.chunk_size
        ));
    }
    let old_base = mem.u64_at(heap_desc + 8)?;
    mem.set_u64(heap_desc + 8, heap.first_chunk_va)?;
    mem.set_u64(heap_desc + 16, heap.first_chunk_va + 64)?;
    mem.set_u64(heap_desc + 24, heap.first_chunk_va + size)?;
    println!(
        "tiler context {tctx0:#x} reset; heap descriptor {heap_desc:#x}: base {old_base:#x} -> {:#x}, bottom +64, top +{size:#x}",
        heap.first_chunk_va
    );
    let mut ok = true;
    for (what, off, want) in [
        ("base", 8, heap.first_chunk_va),
        ("bottom", 16, heap.first_chunk_va + 64),
        ("top", 24, heap.first_chunk_va + size),
    ] {
        let got = mem.u64_at(heap_desc + off)?;
        if got != want {
            println!("heap descriptor {what} reads back {got:#x}, patched to {want:#x}");
            ok = false;
        }
    }

    // The context init stream, with our heap context.
    let mut init = init;
    let mut patched = 0;
    let mut old_ctx = 0;
    for w in init.iter_mut() {
        if let Some((r, v)) = move48_imm(*w) {
            if r == REG_HEAP_CTX {
                *w = cs::move48(REG_HEAP_CTX as u8, heap.ctx_va)?;
                println!("init: MOVE48 d72 {v:#x} -> {:#x}", heap.ctx_va);
                old_ctx = v;
                patched += 1;
            }
        }
    }
    if patched != 1 {
        return refuse(format!("init stream: {patched} MOVE48 d72, expected 1"));
    }
    let shifted = old_ctx != heap.ctx_va && old_base != heap.first_chunk_va;
    if shift_heap && !shifted {
        return refuse(format!(
            "--shift-heap: the real heap did not move (ctx {old_ctx:#x} -> {:#x}, first chunk {old_base:#x} -> {:#x})",
            heap.ctx_va, heap.first_chunk_va
        ));
    }
    println!(
        "heap patch path: {} (ctx {old_ctx:#x} -> {:#x}, first chunk {old_base:#x} -> {:#x})",
        if shifted { "EXERCISED, the VAs differ" } else { "NOT exercised, the kernel returned the trace's VAs" },
        heap.ctx_va,
        heap.first_chunk_va
    );
    let mut init_bo = Bo::new(&vm, "init stream", 4096, Map::Data, 4096)?;
    let bytes: Vec<u8> = init.iter().flat_map(|w| w.to_le_bytes()).collect();
    init_bo.write(0, &bytes)?;
    for (i, w) in init.iter().enumerate() {
        println!("  init[{i}] {w:016x}");
    }

    let lin = trace
        .render_targets
        .iter()
        .rev()
        .find(|r| !r.afbc)
        .ok_or_else(|| Error::Refused("no linear render target to count".into()))?
        .clone();
    let (w, h) = *trace.framebuffers.last().ok_or_else(|| Error::Refused("no framebuffer size".into()))?;
    let stride = lin.row_stride.ok_or_else(|| Error::Refused("linear target without a row stride".into()))? as usize;
    let count = |mem: &mut Mem<'_>| -> Result<PixelCount> {
        let (bo, off) = mem.bo_of(lin.base)?;
        let bytes = bo.bytes()[off..].to_vec();
        count_rgba8(&bytes, w as usize, h as usize, stride).map_err(Error::Refused)
    };
    let poison = count(&mut mem)?;
    println!("linear target {:#x} ({w}x{h}, stride {stride}) before: {poison:?}", lin.base);

    fence(Ordering::SeqCst);
    let f = group.submit(init_bo.va(), bytes.len() as u32)?;
    wait_job(&group, &vm, "init", &f)?;
    drop(f);
    if skip_frame {
        println!("job 0 (frame): not submitted (--skip-frame)");
    } else {
        let f = group.submit(at[0].va, at[0].size)?;
        wait_job(&group, &vm, "job 0 (frame)", &f)?;
        drop(f);
    }
    fence(Ordering::SeqCst);
    let mut afbc_diff = 0;
    for rt in trace.render_targets.iter().filter(|r| r.afbc) {
        let (bo, _) = mem.bo_of(rt.base)?;
        let va = bo.va();
        let mine = bo.bytes().to_vec();
        let d = diff_bytes(&mine, dump_at(&jobs[0].dump, "first", va)?);
        afbc_diff += d;
        println!(
            "AFBC target {:#x} (buffer {va:#x}, {} bytes) after job 0: {d} bytes differ from the first dump",
            rt.base,
            mine.len()
        );
    }

    // Job 1: its own stream and descriptors from the second dump.
    let names: Vec<u64> = jobs[1].cs.iter().filter_map(|w| move48_imm(w.word).map(|(_, v)| v)).collect();
    let mut reloaded = Vec::new();
    for (&va, b1) in &jobs[1].dump {
        let b0 = dump_at(&jobs[0].dump, "first", va)?;
        let size = b1.len() as u64;
        let named = va == at[1].va || names.iter().any(|&n| n >= va && n < va + size);
        if b0 != b1.as_slice() && named && !rt_bufs.contains(&va) && !(heap_desc >= va && heap_desc < va + size) {
            mem.load(va, b1)?;
            reloaded.push(va);
        }
    }
    println!("job 1: reloaded from the second dump {:x?}", reloaded);
    let tctx1 = tiler_ctx(&jobs[1].cs).ok_or_else(|| Error::Refused("job 1: no MOVE48 d40 before RUN_IDVS".into()))?;
    reset_tiler_ctx(&mut mem, tctx1)?;
    let heap_ptr1 = mem.u64_at(tctx1 + 24)?;
    println!(
        "tiler context {tctx1:#x} reset; its heap pointer {heap_ptr1:#x} {} the patched descriptor {heap_desc:#x}",
        if heap_ptr1 == heap_desc { "is" } else { "IS NOT" }
    );
    ok &= heap_ptr1 == heap_desc;
    fence(Ordering::SeqCst);
    let f = group.submit(at[1].va, at[1].size)?;
    wait_job(&group, &vm, "job 1 (read-back blit)", &f)?;
    drop(f);
    fence(Ordering::SeqCst);

    let got = count(&mut mem)?;
    let lin_buf = mem.bo_of(lin.base)?.0.va();
    let reference = count_rgba8(
        &dump_at(&jobs[1].dump, "second", lin_buf)?[(lin.base - lin_buf) as usize..],
        w as usize,
        h as usize,
        stride,
    )
    .map_err(Error::Refused)?;
    println!("pixels (replay):            red {} blue {} other {}", got.red, got.blue, got.other);
    println!("pixels (the trace's dump):  red {} blue {} other {}", reference.red, reference.blue, reference.other);
    let lin_bytes = mem
        .bos
        .get_mut(&lin_buf)
        .ok_or_else(|| Error::Refused(format!("linear target buffer {lin_buf:#x} is not bound")))?
        .bytes()
        .to_vec();
    let d = diff_bytes(&lin_bytes, dump_at(&jobs[1].dump, "second", lin_buf)?);
    println!("linear target buffer {lin_buf:#x} ({} bytes): {d} bytes differ from the second dump", lin_bytes.len());
    if skip_frame {
        return Ok(got.red == 0 && got.blue == 0 && poison.red == 0 && poison.blue == 0);
    }
    Ok(got == reference && d == 0 && afbc_diff == 0 && ok && poison.red == 0 && poison.blue == 0)
}

/// The number of differing bytes (a length difference counts in full).
fn diff_bytes(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count() + a.len().abs_diff(b.len())
}

fn main() -> ExitCode {
    println!("panthor-replay: build {}", yakogl_mali_panthor::BUILD);
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut skip_frame = false;
    let mut shift_heap = false;
    while let Some(flag) = args.first().filter(|a| a.starts_with("--")).cloned() {
        match flag.as_str() {
            "--skip-frame" => skip_frame = true,
            "--shift-heap" => shift_heap = true,
            _ => {
                eprintln!("panthor-replay: unknown option {flag}");
                return ExitCode::from(2);
            }
        }
        args.remove(0);
    }
    if skip_frame {
        println!("panthor-replay: control run, the frame job is not submitted");
    }
    let Some(trace) = args.first() else {
        eprintln!(
            "usage: panthor-replay [--skip-frame] [--shift-heap] <pandecode trace with dumps> [/dev/dri/renderD128]"
        );
        return ExitCode::from(2);
    };
    let dev = args.get(1).cloned().unwrap_or_else(|| "/dev/dri/renderD128".into());
    let what = match (skip_frame, shift_heap) {
        (true, _) => "control (no red, no blue without the frame)",
        (false, true) => "replay with shifted heap VAs",
        (false, false) => "replay",
    };
    match run(trace, &dev, skip_frame, shift_heap) {
        Ok(true) => {
            println!("panthor-replay: {what} PASS");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("panthor-replay: {what} FAIL");
            ExitCode::FAILURE
        }
        Err(e) => {
            println!("panthor-replay: ERROR {e}");
            ExitCode::FAILURE
        }
    }
}
