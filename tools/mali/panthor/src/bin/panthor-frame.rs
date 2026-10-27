// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! panthor-frame: milestone L1 of driver/docs/nanopc-t6-mali.md. YakoGL's
//! own frame (src/mali/frame.rs) run through panthor: no Mesa, no trace
//! bytes but the two shader programs (`tri_shaders`, Mesa 25.0.7's, until
//! our Valhall back end makes them).
//!
//! It creates a VM, a group with one queue and a tiler heap; makes one
//! buffer per arena and builds the descriptors and the command stream in
//! place, each arena based at its buffer's GPU VA ("What L1 must provide");
//! builds the tiler heap descriptor and the sample positions once, from the
//! heap's first chunk VA and chunk size the kernel returned; takes the user
//! register count from the CSIF query and `max_levels` from TILER_FEATURES;
//! submits the queue-init stream (`HEAP_SET` of the kernel's heap context,
//! without Mesa's TILER_OOM handler, which is not built) and waits for it,
//! then the frame, and waits for its syncobj; reads the whole 64x64 target
//! back and counts it. A wait that times out or a group the kernel marks
//! faulted is an error, never a pass.
//!
//! Usage: panthor-frame [--no-draw] [--skip-submit] [--fb /dev/fb0] [--scale N] [/dev/dri/renderD128]
//!
//! - default: clear blue, the two red triangles: expect red 2048, blue
//!   2048, other 0, nothing misplaced (red is the left half).
//! - `--no-draw`: the frame without the draw (clear only): blue 4096.
//! - `--skip-submit`: the init stream runs, the frame is built and not
//!   submitted: the target keeps the fill pattern (fill 4096).
//! - `--fb <path>`: after the read-back, write the target scaled by N
//!   (default 8) to the top-left of a 32-bpp XRGB8888 framebuffer inside a
//!   1-pixel white border, the rest untouched, and print the rectangle.

use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;
use std::sync::atomic::{fence, Ordering};
use std::time::Duration;
use yakogl::mali::arena::Arena;
use yakogl::mali::cs::{Instr, Stream};
use yakogl::mali::frame;
use yakogl_mali_panthor::dev::{Bo, Device, Error, Fence, Group, Map, Result, TilerHeap, Vm, Waited};
use yakogl_mali_panthor::l1::{self, FrameArenas, Tally};

const TIMEOUT: Duration = Duration::from_secs(5);
/// Mesa's tiler heap: 2 MiB chunks, 5 initial, 64 at most, 65535 in flight
/// (driinfo_panfrost.h:16-21, Mesa 25.0.7 pan_csf.c:1420-1426).
const HEAP_CHUNK: u32 = 2 << 20;
const HEAP_INITIAL: u32 = 5;
const HEAP_MAX: u32 = 64;
const HEAP_IN_FLIGHT: u32 = 65535;
/// The arenas' buffers.
const DESC_BYTES: u64 = 16 << 10;
const CODE_BYTES: u64 = 4 << 10;
const SPD_BYTES: u64 = 4 << 10;
const DATA_BYTES: u64 = 4 << 10;
const CONTEXT_BYTES: u64 = 4 << 10;
const CS_WORDS: usize = 512;
const INIT_WORDS: usize = 16;

struct Opts {
    draw: bool,
    submit: bool,
    fb: Option<String>,
    scale: u32,
    dev: String,
}

fn refuse<T>(s: String) -> Result<T> {
    Err(Error::Refused(s))
}

fn fe(e: frame::FrameError) -> Error {
    Error::Refused(format!("frame builder: {e}"))
}

fn ae(e: yakogl::mali::arena::ArenaError) -> Error {
    Error::Refused(format!("arena: {e}"))
}

fn wait_job(group: &Group<'_>, vm: &Vm<'_>, name: &str, f: &Fence<'_>) -> Result<()> {
    let (w, st) = group.wait(f, TIMEOUT)?;
    match w {
        Waited::Signalled(d) => {
            let usable = vm.usable()?;
            println!(
                "{name}: fence signalled, wait {} us, submit to wake {} us; group state {:#x}, fatal queues {:#x}; vm usable {usable}",
                d.as_micros(),
                f.submitted.elapsed().as_micros(),
                st.state,
                st.fatal_queues
            );
            if !usable {
                return refuse(format!(
                    "{name}: the VM is no longer usable after the job (the kernel reports the VM unusable)"
                ));
            }
        }
        Waited::TimedOut => return refuse(format!("{name}: fence not signalled within {} s", TIMEOUT.as_secs())),
    }
    Ok(())
}

/// Write the CS words into a buffer (their addresses are final, so they are
/// the one thing that may be copied rather than built in place).
fn put_words(bo: &mut Bo<'_>, words: &[u64]) -> Result<()> {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    bo.write(0, &bytes)
}

fn describe(label: &str, words: &[u64]) {
    for (i, &w) in words.iter().enumerate() {
        let name = Instr::decode(w).map(|i| i.name()).unwrap_or("UNDECODED");
        println!("  {label}[{i:2}] {w:016x} {name}");
    }
}

fn high_water(a: &Arena<'_>) {
    println!("arena '{}': high-water {} of {} bytes", a.label(), a.high_water(), a.capacity());
}

/// A sysfs attribute of the framebuffer device `fb` (e.g. "fb0").
fn fb_attr(fb: &str, name: &str) -> Result<String> {
    let p = format!("/sys/class/graphics/{fb}/{name}");
    std::fs::read_to_string(&p)
        .map(|s| s.trim().to_string())
        .map_err(|err| Error::Os { what: format!("read {p}"), err })
}

fn blit(path: &str, target: &[u8], scale: u32) -> Result<()> {
    let fb = path.rsplit('/').next().unwrap_or(path);
    let bpp = fb_attr(fb, "bits_per_pixel")?;
    let stride: u32 = fb_attr(fb, "stride")?.parse().map_err(|_| Error::Refused(format!("{fb}: stride")))?;
    let vsize = fb_attr(fb, "virtual_size")?;
    let pan = fb_attr(fb, "pan").unwrap_or_else(|_| "absent".into());
    let name = fb_attr(fb, "name").unwrap_or_else(|_| "absent".into());
    println!("fb {path}: name {name}, {bpp} bpp, stride {stride}, virtual_size {vsize}, pan {pan}");
    if bpp != "32" {
        return refuse(format!("{path}: {bpp} bits per pixel, the blit writes 32-bpp XRGB8888 only"));
    }
    let (w, h) = vsize
        .split_once(',')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .ok_or_else(|| Error::Refused(format!("{fb}: virtual_size {vsize:?}")))?;
    let (rect, rows) = l1::fb_rows(target, scale, w, h, stride).map_err(Error::Refused)?;
    let f = OpenOptions::new().write(true).open(path).map_err(|err| Error::Os { what: format!("open {path}"), err })?;
    let mut bytes = 0;
    for (off, row) in &rows {
        f.write_all_at(row, *off as u64).map_err(|err| Error::Os { what: format!("write {path} at {off:#x}"), err })?;
        bytes += row.len();
    }
    println!(
        "fb {path}: wrote x [{}, {}) y [{}, {}) ({}x{} pixels, {} rows, {bytes} bytes): the 64x64 target at {scale}x inside a 1-pixel white border; nothing else written",
        rect.x0,
        rect.x1,
        rect.y0,
        rect.y1,
        rect.x1 - rect.x0,
        rect.y1 - rect.y0,
        rows.len()
    );
    Ok(())
}

fn run(o: &Opts) -> Result<bool> {
    let dev = Device::open(&o.dev)?;
    let c = &dev.csif;
    println!(
        "device {}: {} {}.{}.{}, gpu_id {:#010x}, tiler_features {:#010x}; csif cs_reg_count {} unpreserved {} scoreboards {}",
        o.dev,
        dev.driver.name,
        dev.driver.major,
        dev.driver.minor,
        dev.driver.patch,
        dev.gpu.gpu_id,
        dev.gpu.tiler_features,
        c.cs_reg_count,
        c.unpreserved_cs_reg_count,
        c.scoreboard_slot_count
    );
    let user_regs = l1::user_regs(c.cs_reg_count, c.unpreserved_cs_reg_count)
        .ok_or_else(|| Error::Refused(format!("CSIF: {} registers leave none to the user", c.cs_reg_count)))?;
    /* TILER_FEATURES bits 11:8, max_levels (Mesa pan_props.c:116) */
    let max_levels = (dev.gpu.tiler_features >> 8) & 0xf;
    println!("user registers {user_regs}, tiler max_levels {max_levels}");

    let vm = Vm::create(&dev)?;
    let group = Group::new(&vm, 64 * 1024)?;
    let heap = TilerHeap::new(&vm, HEAP_CHUNK, HEAP_INITIAL, HEAP_MAX, HEAP_IN_FLIGHT)?;
    println!(
        "vm {} (user VA {:#x}), group {}, tiler heap {}: ctx {:#x}, first chunk {:#x}, chunk {:#x}, {} initial / {} max",
        vm.id,
        vm.user_va_range,
        group.handle,
        heap.handle,
        heap.ctx_va,
        heap.first_chunk_va,
        heap.chunk_size,
        heap.initial_chunks,
        heap.max_chunks
    );

    let mut ctx_bo = Bo::new(&vm, "context", CONTEXT_BYTES, Map::Data, 4096)?;
    let geom = Bo::new(&vm, "geometry buffer", u64::from(l1::GEOMETRY_BUFFER_BYTES), Map::Data, 4096)?;
    let mut desc_bo = Bo::new(&vm, "desc", DESC_BYTES, Map::Data, 4096)?;
    let mut code_bo = Bo::new(&vm, "code", CODE_BYTES, Map::Code, 4096)?;
    let mut spd_bo = Bo::new(&vm, "spds", SPD_BYTES, Map::Data, 4096)?;
    let mut data_bo = Bo::new(&vm, "data", DATA_BYTES, Map::Data, 4096)?;
    let mut target_bo = Bo::new(&vm, "target", u64::from(l1::ROW_STRIDE * l1::HEIGHT), Map::Data, 4096)?;
    let mut cs_bo = Bo::new(&vm, "frame stream", (CS_WORDS * 8) as u64, Map::Data, 4096)?;
    let mut init_bo = Bo::new(&vm, "init stream", (INIT_WORDS * 8) as u64, Map::Data, 4096)?;
    for b in [&ctx_bo, &geom, &desc_bo, &code_bo, &spd_bo, &data_bo, &target_bo, &cs_bo, &init_bo] {
        println!("bo {:16} va {:#014x} size {:#x}", b.label, b.va(), b.size);
    }

    /* the per-context objects, built once */
    let (ctx_va, desc_va, code_va, spd_va, data_va) =
        (ctx_bo.va(), desc_bo.va(), code_bo.va(), spd_bo.va(), data_bo.va());
    let mut ctx_arena = Arena::new("context", ctx_va, ctx_bo.bytes()).map_err(ae)?;
    let ctx =
        l1::build_context(&mut ctx_arena, heap.first_chunk_va, heap.chunk_size, geom.va(), max_levels).map_err(fe)?;
    println!(
        "context: heap descriptor {:#x} (first chunk {:#x}, size {:#x}), sample positions {:#x}, geometry buffer {:#x} ({} bytes)",
        ctx.heap, heap.first_chunk_va, heap.chunk_size, ctx.sample_positions, ctx.geometry_buffer, ctx.geometry_buffer_size
    );
    high_water(&ctx_arena);

    /* the queue-init stream */
    let mut init_words = [0u64; INIT_WORDS];
    let mut init = Stream::new(&mut init_words, user_regs);
    frame::queue_init(&mut init, heap.ctx_va).map_err(fe)?;
    let init_w = init.words().to_vec();
    println!("init stream at {:#x}: {} words", init_bo.va(), init_w.len());
    describe("init", &init_w);
    put_words(&mut init_bo, &init_w)?;

    /* the frame, built in place */
    let mut words = [0u64; CS_WORDS];
    let built;
    {
        let mut desc = Arena::new("desc", desc_va, desc_bo.bytes()).map_err(ae)?;
        let mut code = Arena::new("code", code_va, code_bo.bytes()).map_err(ae)?;
        let mut spds = Arena::new("spds", spd_va, spd_bo.bytes()).map_err(ae)?;
        let mut data = Arena::new("data", data_va, data_bo.bytes()).map_err(ae)?;
        let mut cs = Stream::new(&mut words, user_regs);
        let a = FrameArenas { desc: &mut desc, code: &mut code, spds: &mut spds, data: &mut data };
        let f = l1::build_triangle_frame(a, &mut cs, &ctx, target_bo.va(), o.draw).map_err(fe)?;
        built = cs.words().len();
        for a in [&desc, &code, &spds, &data] {
            high_water(a);
        }
        println!("arena 'cs': high-water {} of {} bytes", 8 * built, 8 * CS_WORDS);
        println!(
            "frame: {} the draw, fbd pointer {:#x}, tiler context {}",
            if o.draw { "with" } else { "without" },
            f.fbd_pointer,
            f.layout.tiler.map_or("none".to_string(), |b| format!("{:#x}", b.gpu))
        );
    }
    let frame_words = &words[..built];
    println!("frame stream at {:#x}: {} words", cs_bo.va(), frame_words.len());
    describe("frame", frame_words);
    put_words(&mut cs_bo, frame_words)?;

    /* the target holds the fill pattern before anything runs */
    for p in target_bo.bytes().chunks_mut(4) {
        p.copy_from_slice(&l1::FILL);
    }
    let before = l1::tally(target_bo.bytes(), o.draw).map_err(Error::Refused)?;
    println!("target {:#x} before: {}", target_bo.va(), show(&before));

    fence(Ordering::SeqCst);
    let f = group.submit(init_bo.va(), (init_w.len() * 8) as u32)?;
    wait_job(&group, &vm, "init", &f)?;
    drop(f);
    if o.submit {
        let f = group.submit(cs_bo.va(), (frame_words.len() * 8) as u32)?;
        wait_job(&group, &vm, "frame", &f)?;
        drop(f);
    } else {
        println!("frame: not submitted (--skip-submit)");
    }
    fence(Ordering::SeqCst);

    let pixels = target_bo.bytes().to_vec();
    let after = l1::tally(&pixels, o.draw).map_err(Error::Refused)?;
    println!("target {:#x} after:  {}", target_bo.va(), show(&after));
    /* the heap descriptor as the GPU left it (it moves Bottom and Top) */
    let off = (ctx.heap - ctx_va) as usize;
    let h: Vec<u64> = (0..4).map(|i| ctx_bo.read_u64(off + 8 * i)).collect::<Result<_>>()?;
    println!("heap descriptor after: {:016x} {:016x} {:016x} {:016x}", h[0], h[1], h[2], h[3]);
    /* words 2 and 3 are Bottom and Top (v10 TilerHeap); the descriptor is built
    with both inside the first chunk, so a value outside it means the GPU took
    another chunk (the heap grew) */
    let (lo, hi) = (heap.first_chunk_va, heap.first_chunk_va + u64::from(heap.chunk_size));
    let inside = |v: u64| (lo..=hi).contains(&v);
    println!(
        "heap growth: bottom {:#x}, top {:#x}, first chunk {lo:#x}..{hi:#x}: {}",
        h[2],
        h[3],
        if inside(h[2]) && inside(h[3]) {
            "not seen (both inside the first chunk)"
        } else {
            "seen (left the first chunk)"
        }
    );

    if let Some(fb) = &o.fb {
        blit(fb, &pixels, o.scale)?;
    }
    let ok = match (o.submit, o.draw) {
        (false, _) => after.fill == 4096 && after == before,
        (true, true) => after == Tally { red: 2048, blue: 2048, other: 0, fill: 0, misplaced: 0 },
        (true, false) => after == Tally { red: 0, blue: 4096, other: 0, fill: 0, misplaced: 0 },
    };
    Ok(ok)
}

fn show(t: &Tally) -> String {
    format!("red {} blue {} other {} (fill {}) misplaced {}", t.red, t.blue, t.other, t.fill, t.misplaced)
}

fn main() -> ExitCode {
    println!("panthor-frame: build {}", yakogl_mali_panthor::BUILD);
    let mut o = Opts { draw: true, submit: true, fb: None, scale: 8, dev: "/dev/dri/renderD128".into() };
    let mut args = std::env::args().skip(1);
    let mut dev = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-draw" => o.draw = false,
            "--skip-submit" => o.submit = false,
            "--fb" => match args.next() {
                Some(p) => o.fb = Some(p),
                None => {
                    eprintln!("panthor-frame: --fb takes a framebuffer device path");
                    return ExitCode::from(2);
                }
            },
            "--scale" => match args.next().and_then(|s| s.parse().ok()) {
                Some(n) => o.scale = n,
                None => {
                    eprintln!("panthor-frame: --scale takes a number");
                    return ExitCode::from(2);
                }
            },
            s if s.starts_with("--") => {
                eprintln!("panthor-frame: unknown option {s}");
                return ExitCode::from(2);
            }
            _ => dev = Some(a),
        }
    }
    if let Some(d) = dev {
        o.dev = d;
    }
    let what = match (o.submit, o.draw) {
        (false, _) => "control --skip-submit (expect the fill pattern, fill 4096)",
        (true, false) => "control --no-draw (expect blue 4096)",
        (true, true) => "frame (expect red 2048 blue 2048 other 0 misplaced 0)",
    };
    println!("panthor-frame: {what}");
    match run(&o) {
        Ok(true) => {
            println!("panthor-frame: {what}: PASS");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("panthor-frame: {what}: FAIL");
            ExitCode::FAILURE
        }
        Err(e) => {
            println!("panthor-frame: {what}: ERROR {e}");
            ExitCode::FAILURE
        }
    }
}
