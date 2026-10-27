// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! panthor-hello: the smallest job that proves the submit path. It submits a
//! command stream that only adds a constant to a 64-bit sync object in a
//! buffer it owns (`SYNC_ADD64`), closed with Mesa's batch end, and proves
//! completion by reading the value back after the fence.
//!
//! The stream (cs.rs has each encoding and its v10.xml line):
//!
//! ```text
//! MOVE48 d0, #<sync VA>          the address, register pair r0:r1
//! MOVE48 d2, #INCREMENT          the 64-bit addend, r2:r3
//! SYNC_ADD64 [d0] += d2          system scope, error propagate, synchronous
//! WAIT #ff                       \
//! MOVE32 r74, #0                  | Mesa's csf_emit_batch_end
//! FLUSH_CACHE2 clean/clean/inv    | (pan_csf.c:556-605)
//! WAIT #1                        /
//! ```
//!
//! The kernel wraps its own stream around it (a cache flush before, then a
//! CALL, then its own SYNC_ADD64 on the queue's sync object that signals the
//! fence; panthor_sched.c:3040-3094), so the fence alone would signal even
//! if our instructions did nothing: the value read back is the evidence.
//!
//! Usage: panthor-hello [--control] [/dev/dri/renderD128]. Exit 0 only when
//! the value read back is exactly the start value plus the increment, the
//! status word reads 0, the pad survives, and the group reports no timeout
//! and no fault. `--control` submits the same stream with the SYNC_ADD64
//! replaced by a NOP and expects all 16 bytes unchanged: it shows that the
//! read-back can tell the instruction ran from the job merely completing.

use std::process::ExitCode;
use std::sync::atomic::{fence, Ordering};
use std::time::Duration;
use yakogl_mali_panthor::cs::{self, Async, Stream, SyncScope};
use yakogl_mali_panthor::dev::{Bo, Device, Group, Map, Result, TilerHeap, Vm, Waited};

/// A start value no fresh, zeroed or stale buffer holds, and an addend that
/// a "wrote 1" or "wrote the addend" result cannot imitate.
const START: u64 = 0x0123_4567_89ab_0000;
const INCREMENT: u64 = 0x2a;
/// The rest of the 16-byte sync object: a u32 status, "not zero on failure",
/// and a u32 pad (`struct panthor_syncobj_64b`, Linux v6.18
/// panthor_sched.c:338-351). The status starts non-zero, so the GPU's
/// error-propagating SYNC_ADD64 writing 0 there (seen on the first board run,
/// 2026-10-18) is a second reading of "done without error"; the pad must
/// survive.
const STATUS_START: u32 = 0xa5a5_a5a5;
const PAD: u32 = 0x5a5a_5a5a;
const TIMEOUT: Duration = Duration::from_secs(5);

fn run(path: &str, control: bool) -> Result<bool> {
    let dev = Device::open(path)?;
    dev.print_info();
    let vm = Vm::create(&dev)?;
    println!("vm {}: user VA range {:#x}, usable {}", vm.id, vm.user_va_range, vm.usable()?);
    let group = Group::new(&vm, 64 * 1024)?;
    println!("group {}: one queue, ring {} KiB", group.handle, group.ringbuf_size / 1024);
    // Not used by this stream: created to prove the ioctl and print the
    // addresses the kernel chose (Mesa's defaults, pan_csf.c:1854-1860).
    let heap = TilerHeap::new(&vm, 2 << 20, 5, 64, 65535)?;
    println!(
        "tiler heap {}: context at {:#x}, first chunk at {:#x} ({} KiB chunks, {} initial, {} max)",
        heap.handle,
        heap.ctx_va,
        heap.first_chunk_va,
        heap.chunk_size / 1024,
        heap.initial_chunks,
        heap.max_chunks
    );

    let mut sync = Bo::new(&vm, "sync", 4096, Map::Data, 4096)?;
    let mut stream_bo = Bo::new(&vm, "stream", 4096, Map::Data, 4096)?;
    println!("bo sync: handle {} at {:#x}, {} bytes", sync.handle, sync.va(), sync.size);
    println!("bo stream: handle {} at {:#x}, {} bytes", stream_bo.handle, stream_bo.va(), stream_bo.size);

    let c = &dev.csif;
    let mut s = Stream::new("hello", c.cs_reg_count, c.unpreserved_cs_reg_count, c.scoreboard_slot_count)?;
    s.push(cs::move48(0, sync.va()))?;
    s.push(cs::move48(2, INCREMENT))?;
    if control {
        s.push(Ok(cs::nop()))?;
    } else {
        s.push(cs::sync_add64(true, SyncScope::System, 2, 0, Async::Now))?;
    }
    s.push_mesa_batch_end()?;
    println!("stream ({} instructions at {:#x}):", s.words().len(), stream_bo.va());
    for (i, w) in s.words().iter().enumerate() {
        println!("  [{i}] {w:016x}  {}", cs::describe(*w));
    }
    stream_bo.write(0, &s.bytes())?;

    sync.write_u64(0, START)?;
    sync.write_u64(8, (PAD as u64) << 32 | STATUS_START as u64)?;
    // The CPU mapping is write-combined: order the stores before the submit.
    fence(Ordering::SeqCst);
    let before = (sync.read_u64(0)?, sync.read_u64(8)?);
    println!("sync before: value {:#018x}, status {:#010x}, pad {:#010x}", before.0, before.1 as u32, before.1 >> 32);

    let f = group.submit(stream_bo.va(), s.bytes().len() as u32)?;
    println!("submit: ioctl {} us", f.ioctl_time.as_micros());
    let (waited, state) = group.wait(&f, TIMEOUT)?;
    let since_submit = f.submitted.elapsed();
    match waited {
        Waited::Signalled(d) => {
            println!("fence: signalled; wait call {} us, submit to wake {} us", d.as_micros(), since_submit.as_micros())
        }
        Waited::TimedOut => println!("fence: NOT signalled within {} s", TIMEOUT.as_secs()),
    }
    println!("group state {:#x}, fatal queues {:#x}", state.state, state.fatal_queues);
    fence(Ordering::SeqCst);
    let after = (sync.read_u64(0)?, sync.read_u64(8)?);
    let (want, want_status) = if control { (START, STATUS_START) } else { (START + INCREMENT, 0) };
    let (status, pad) = (after.1 as u32, (after.1 >> 32) as u32);
    println!(
        "sync after: value {:#018x} (expected {want:#018x}), status {status:#010x} (expected {want_status:#010x}), pad {pad:#010x} (expected {PAD:#010x})",
        after.0
    );
    println!("vm usable after the job: {}", vm.usable()?);
    Ok(waited != Waited::TimedOut && after.0 == want && status == want_status && pad == PAD)
}

fn main() -> ExitCode {
    println!("panthor-hello: build {}", yakogl_mali_panthor::BUILD);
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let control = args.first().is_some_and(|a| a == "--control");
    if control {
        args.remove(0);
        println!("panthor-hello: control run, SYNC_ADD64 replaced by NOP");
    }
    let path = args.first().cloned().unwrap_or_else(|| "/dev/dri/renderD128".into());
    let what = if control { "control (no change without SYNC_ADD64)" } else { "SYNC_ADD64" };
    match run(&path, control) {
        Ok(true) => {
            println!("panthor-hello: {what} PASS");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("panthor-hello: {what} FAIL");
            ExitCode::FAILURE
        }
        Err(e) => {
            println!("panthor-hello: ERROR {e}");
            ExitCode::FAILURE
        }
    }
}
