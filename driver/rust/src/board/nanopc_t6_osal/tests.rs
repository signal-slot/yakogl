// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The OSAL over the Z4 MCU model (nanopc_t6_submit's `Z4`: Z3's T6 with
//! the synthetic firmware, and the firmware's side of CSG 0 / CS 0, the
//! ring and the sync object), first op by op, then with the GL stack on
//! top: the L2 triangle (`the_l2_triangle_draws_through_the_osal`).

use super::*;
use crate::board::nanopc_t6_fw::tests::{backings, B, TABLES_PA};
use crate::board::nanopc_t6_submit::tests::{need_chunks, Fail, VM_TABLES_PA, Z4};
use crate::board::nanopc_t6_submit::{BLUE, RED, STEP_NONE};
use crate::gpu::mali::csf::RK3588_TIMER_HZ;
use crate::gpu::mali::fw::synth;
use crate::gpu::mali::heap::chunk_link;
use crate::gpu::mali::pgtable::ENTRIES;
use crate::gpu::mali::sched::job::{mov48, sync_add64};
use crate::gpu::mali::sched::RING_MIN_SIZE;

/// The carve-out of the tests: 16 MiB, the backend's two frame slots
/// (about 1.8 MiB each) and the GL objects with room to spare.
const CARVE: u64 = 16 << 20;

/// The model, its OSAL and the table pools both live on.
struct Rig {
    z: Box<Z4>,
    osal: Box<Osal>,
    o: SubmitUp,
    _pool: Vec<Table>,
    _vm_pool: Vec<Table>,
}

fn rig() -> Rig {
    rig_with(1, 1)
}

/// The rig with `chunks` chunk slots, `initial` of them the heap's at the
/// start.
fn rig_with(chunks: u64, initial: u32) -> Rig {
    let blob = synth::small();
    let z = Box::new(Z4::with_regions(
        blob.clone(),
        &[
            (QUEUE_VA, QUEUE_VM_BYTES),
            (HEAP_POOL_VA, HEAP_POOL_SIZE),
            (HEAP_CHUNK_VA, chunks * u64::from(HEAP_CHUNK_SIZE)),
            (CARVE_VA, CARVE),
        ],
    ));
    let vm = z.backings();
    let mut pool = vec![[0u64; ENTRIES]; 16];
    let mut vm_pool = vec![[0u64; ENTRIES]; 32];
    let mut osal = Box::new(Osal::EMPTY);
    let mut o = SubmitUp::default();
    let mem = OsalMem {
        queue: crate::board::nanopc_t6_submit::tests::QUEUE,
        ring: vm[0],
        heap_pool: vm[1],
        heap_chunks: vm[2],
        carve: vm[3],
        vm_pool: &mut vm_pool,
        vm_tables_pa: VM_TABLES_PA,
        heap_initial_chunks: initial,
    };
    let r = osal.up(
        &*z,
        B,
        &blob,
        &backings(&blob),
        &mut pool,
        TABLES_PA,
        RK3588_TIMER_HZ,
        mem,
        &|a, n| z.t6.clean(a, n),
        &mut |_| {},
        &mut |_| {},
        &mut GpuUp::default(),
        &mut o,
    );
    assert_eq!((r, o.step, o.refusal, o.detail), (Ok(()), STEP_NONE, 0, 0));
    Rig { z, osal, o, _pool: pool, _vm_pool: vm_pool }
}

fn no_delay(_: u32) {}

/// The group comes up as Z4's does, over the four regions, and the two
/// queries answer the device and the heap.
#[test]
fn the_osal_comes_up_and_answers_the_two_queries() {
    let r = rig();
    assert!(r.osal.is_up());
    // the group's start as Z4's record has it (nanopc_t6_submit's test)
    assert_eq!((r.o.csg_req, r.o.csg_ack, r.o.cs_req, r.o.cs_ack), (0x11, 0x11, 0x0511, 0x0011));
    // 4 KiB pages only: L0, the L1 of the low 512 GiB, the queue's L2 and
    // L3; the heap's L1, L2 and the pool's L3, the chunk's L3; the
    // carve-out's L2 and its eight L3s (16 MiB)
    assert_eq!(r.o.vm_tables_used, 17);
    let g = r.osal.gpu_info().unwrap();
    assert_eq!(g.gpu_id, crate::gpu::mali::G610_GPU_ID);
    // the G610's 96 registers of which the firmware keeps 92 (csf.rs's test)
    assert_eq!((g.cs_reg_count, g.unpreserved_cs_reg_count), (96, 4));
    let h = r.osal.heap().unwrap();
    assert_eq!((h.ctx_gpu, h.first_chunk_gpu, h.chunk_size), (HEAP_POOL_VA, HEAP_CHUNK_VA, HEAP_CHUNK_SIZE));
    assert_eq!(r.osal.fences(), (0, 0));
}

/// Pages come first fit from the carve-out's start; a free is held until
/// the kick being recorded completes, and its pages are taken again only
/// then.
#[test]
fn allocations_are_carve_pages_and_a_free_waits_for_the_kick_being_recorded() {
    let mut r = rig();
    let a = r.osal.alloc(100).unwrap();
    assert_eq!((a.gpu, a.size, a.handle), (CARVE_VA, 4096, 1));
    let b = r.osal.alloc(5000).unwrap();
    assert_eq!((b.gpu, b.size, b.handle), (CARVE_VA + 0x1000, 8192, 2));
    assert_eq!((b.cpu - a.cpu, b.phys - a.phys), (0x1000, 0x1000));
    assert_eq!(r.osal.alloc(0), Err(-EINVAL));
    assert_eq!(r.osal.alloc(CARVE as usize + 1), Err(-ENOMEM));

    // freed while kick 1 is being recorded: held
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), -EINVAL, "a second free of the same pages");
    assert_eq!(r.osal.free(&*r.z, b.gpu + 0x1000, 1, &mut no_delay), -EINVAL, "not an allocation's first page");
    assert_eq!(r.osal.held(), 1);
    let c = r.osal.alloc(1).unwrap();
    assert_eq!(c.gpu, CARVE_VA + 0x3000, "page 0 is still held");

    // kick 1 recorded and completed: the pages go back
    let s = stream(&mut r);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(f, 1);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!(r.osal.held(), 0);
    assert_eq!(r.osal.alloc(1).unwrap().gpu, CARVE_VA);
    assert_eq!(r.osal.live_pages, 5);
}

/// A 24-byte stream in the carve-out that adds 1 to the word 32 bytes from
/// its start: `MOV48 d0, word`, `MOV48 d2, 1`, `SYNC_ADD64 d0, d2`.
fn stream(r: &mut Rig) -> Alloc {
    let s = r.osal.alloc(64).unwrap();
    let word = s.gpu + 32;
    for (i, w) in [mov48(0, word), mov48(2, 1), sync_add64(0, 2), 0].iter().enumerate() {
        // SAFETY: the allocation's CPU view, 4 KiB, the model's RAM
        unsafe { ((s.cpu + 8 * i) as *mut u64).write_volatile(*w) };
    }
    s
}

fn word(s: &Alloc) -> u64 {
    // SAFETY: as in `stream`
    unsafe { ((s.cpu + 32) as *const u64).read_volatile() }
}

/// Forty kicks, more than the 4 KiB ring holds (32 jobs of 128 bytes): the
/// ring wraps, a full ring waits for its oldest job, every fence completes
/// in order, each kick's stream ran once. The model's firmware publishes
/// `extract` as the board's does, late (`PUBLISH_LAG`).
#[test]
fn kicks_run_on_the_ring_and_their_fences_complete() {
    forty_kicks(rig());
}

/// The same forty kicks while the firmware never publishes `extract`: the
/// ring's room comes from the sync object alone.
#[test]
fn a_full_ring_does_not_wait_for_extract() {
    let mut r = rig();
    r.z.publish_lag = Some(u32::MAX);
    let r = forty_kicks(r);
    assert_eq!(r.z.output_extract(), 0, "extract was never published");
}

fn forty_kicks(mut r: Rig) -> Rig {
    let s = stream(&mut r);
    let jobs = RING_MIN_SIZE / 128;
    assert_eq!(jobs, 32);
    for k in 1..=40 {
        assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), k);
    }
    let (sub, done) = r.osal.fences();
    assert_eq!(sub, 40);
    assert!(done >= 40 - u64::from(jobs), "the full ring waited: {done}");
    assert_eq!(r.osal.wait(&*r.z, 40, -1, &mut no_delay), 0);
    assert_eq!(r.osal.completed(&*r.z, &mut no_delay), 40);
    assert_eq!((word(&s), r.z.calls()), (40, 40));
    assert_eq!(r.osal.wait(&*r.z, 7, 0, &mut no_delay), 0, "an older fence");
    assert_eq!(r.osal.wait(&*r.z, 41, -1, &mut no_delay), -EINVAL, "never submitted");
    assert_eq!(r.osal.dead(), None);
    r
}

/// The ring's input and output pages and the sync object, read back.
fn ring_words(r: &Rig) -> (u64, u64, u64) {
    let s = r.osal.up.unwrap().started;
    let rd = |a: usize| u64::from(r.z.read32(a)) | u64::from(r.z.read32(a + 4)) << 32;
    (
        rd(s.input_cpu + core::mem::offset_of!(csf::RingbufInput, insert)),
        rd(s.output_cpu + core::mem::offset_of!(csf::RingbufOutput, extract)),
        rd(s.syncobj_cpu),
    )
}

/// 820 kicks in bursts of 1, 2, ... 40, the ring (32 jobs) wrapping 25
/// times: each burst longer than the ring waits for its oldest job,
/// every kind of wait (panthor's 5 s, a short one, an older fence,
/// `completed`) sees its fence, and a buffer freed during each burst comes
/// back once the burst's first kick completed. Each stream ran once, in
/// order, and the sync object counts every job. The model's firmware
/// publishes `extract` late, as the board's does (`PUBLISH_LAG`); no full
/// ring restarts the group.
#[test]
fn hundreds_of_kicks_wrap_the_ring_and_every_fence_completes_in_order() {
    hundreds_of_kicks(rig());
}

/// The same 820 kicks while the firmware never publishes `extract`.
#[test]
fn hundreds_of_kicks_do_not_wait_for_extract() {
    let mut r = rig();
    r.z.publish_lag = Some(u32::MAX);
    hundreds_of_kicks(r);
}

fn hundreds_of_kicks(mut r: Rig) {
    const KICKS: u64 = 40 * 41 / 2;
    let s = stream(&mut r);
    let mut fence = 0i64;
    let mut burst = 0;
    let mut full_waits = 0;
    while burst < 40 {
        burst += 1;
        let n = burst;
        let held = r.osal.alloc(4096).unwrap();
        assert_eq!(r.osal.free(&*r.z, held.gpu, held.handle, &mut no_delay), 0);
        for _ in 0..n {
            fence += 1;
            assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), fence);
        }
        let first = fence - n + 1;
        if n > 32 {
            let done = r.osal.fences().1 as i64;
            assert!(done >= fence - 32, "a full ring waited for its oldest job: {done} of {fence}");
            full_waits += 1;
        }
        match burst % 4 {
            0 => assert_eq!(r.osal.wait(&*r.z, fence, -1, &mut no_delay), 0),
            1 => assert_eq!(r.osal.wait(&*r.z, fence, 1_000_000, &mut no_delay), 0),
            2 => {
                assert_eq!(r.osal.wait(&*r.z, first, -1, &mut no_delay), 0);
                assert_eq!(r.osal.wait(&*r.z, fence, -1, &mut no_delay), 0);
            }
            _ => while r.osal.completed(&*r.z, &mut no_delay) < fence {},
        }
        assert_eq!(r.osal.completed(&*r.z, &mut no_delay), fence);
        assert_eq!(r.osal.held(), 0, "the burst's free went back at fence {first}");
    }
    assert_eq!(r.osal.fences(), (KICKS, KICKS));
    assert_eq!(full_waits, 8, "bursts 33 to 40");
    assert_eq!((word(&s), u64::from(r.z.calls())), (KICKS, KICKS));
    let (insert, extract, seqno) = ring_words(&r);
    assert_eq!((insert, seqno, r.z.stream_extract()), (KICKS * 128, KICKS, KICKS * 128));
    // the firmware's to publish, later; the OSAL never waited for it
    assert!(extract <= insert, "{extract}");
    assert_eq!((r.osal.restarts, r.osal.last_error()), (0, None));
    // only the stream's page in use; the oldest fence; a fence never submitted
    assert_eq!(r.osal.live_pages, 1);
    assert_eq!(r.osal.wait(&*r.z, 1, -1, &mut no_delay), 0);
    assert_eq!(r.osal.wait(&*r.z, KICKS as i64 + 1, -1, &mut no_delay), -EINVAL);
    assert_eq!(r.osal.dead(), None);
}

/// A stream of `words` in a fresh 4 KiB allocation.
fn stream_of(r: &mut Rig, words: &[u64]) -> Alloc {
    let s = r.osal.alloc(64).unwrap();
    for (i, w) in words.iter().enumerate() {
        // SAFETY: the allocation's CPU view, 4 KiB, the model's RAM
        unsafe { ((s.cpu + 8 * i) as *mut u64).write_volatile(*w) };
    }
    s
}

/// The backend's queue-init stream, Z4's: `HEAP_SET` of the heap context,
/// run and waited for, as the backend's bring-up does.
fn queue_init(r: &mut Rig) {
    let init = stream_of(r, &z4::QUEUE_INIT);
    let f = r.osal.submit(&*r.z, init.gpu, 16, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!(r.z.heap_ctx(), Some(HEAP_POOL_VA));
}

/// A stream that needs `chunks` heap chunks, then adds 1 to its word (32
/// bytes in, as [`stream`]'s).
fn oom_stream(r: &mut Rig, chunks: u32) -> Alloc {
    let s = r.osal.alloc(64).unwrap();
    let words = [need_chunks(chunks), mov48(0, s.gpu + 32), mov48(2, 1), sync_add64(0, 2)];
    for (i, w) in words.iter().enumerate() {
        // SAFETY: as in stream
        unsafe { ((s.cpu + 8 * i) as *mut u64).write_volatile(*w) };
    }
    s
}

/// The heap the OSAL makes is the one group_up made (the query's), its
/// initial chunks first fit in the chunk region.
#[test]
fn the_heap_starts_with_its_initial_chunks_and_answers_group_up_s() {
    let r = rig_with(4, 2);
    let h = r.osal.heap().unwrap();
    let c = HEAP_CHUNK_SIZE as u64;
    assert_eq!((h.ctx_gpu, h.first_chunk_gpu), (HEAP_POOL_VA, HEAP_CHUNK_VA + c));
    let made: Vec<u64> = r.osal.heap_chunks().iter().map(|a| a.gpu).collect();
    assert_eq!(made, [HEAP_CHUNK_VA, HEAP_CHUNK_VA + c]);
    // the second chunk links to the first, as create links them
    // SAFETY: the chunk's CPU view, the model's RAM
    let next = unsafe { (r.osal.heap_chunks()[1].cpu as *const u64).read_volatile() };
    assert_eq!(next, chunk_link(HEAP_CHUNK_VA, HEAP_CHUNK_SIZE));
}

/// No initial chunk, or more than the region holds: refused at the heap
/// step before anything runs.
#[test]
fn an_initial_chunk_count_the_region_cannot_hold_is_refused() {
    for (chunks, initial, code) in [(2, 0, 1), (2, 3, 2)] {
        let blob = synth::small();
        let z = Z4::with_regions(
            blob.clone(),
            &[
                (QUEUE_VA, QUEUE_VM_BYTES),
                (HEAP_POOL_VA, HEAP_POOL_SIZE),
                (HEAP_CHUNK_VA, chunks * u64::from(HEAP_CHUNK_SIZE)),
                (CARVE_VA, CARVE),
            ],
        );
        let vm = z.backings();
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut vm_pool = vec![[0u64; ENTRIES]; 32];
        let mem = OsalMem {
            queue: crate::board::nanopc_t6_submit::tests::QUEUE,
            ring: vm[0],
            heap_pool: vm[1],
            heap_chunks: vm[2],
            carve: vm[3],
            vm_pool: &mut vm_pool,
            vm_tables_pa: VM_TABLES_PA,
            heap_initial_chunks: initial,
        };
        let (mut osal, mut o) = (Box::new(Osal::EMPTY), SubmitUp::default());
        let r = osal.up(
            &z,
            B,
            &blob,
            &backings(&blob),
            &mut pool,
            TABLES_PA,
            RK3588_TIMER_HZ,
            mem,
            &|a, n| z.t6.clean(a, n),
            &mut |_| {},
            &mut |_| {},
            &mut GpuUp::default(),
            &mut o,
        );
        assert_eq!((r, o.refusal, o.detail), (Err(z4::STEP_HEAP), code, u64::from(initial)));
        assert!(!osal.is_up());
    }
}

/// A job that needs two chunks more than the heap starts with: two
/// `CS_TILER_OOM`s, each answered with the next free chunk (its link in
/// `heap_start`, as panthor's handler writes it), and the job completes.
#[test]
fn a_tiler_oom_grows_the_heap_and_the_job_completes() {
    let mut r = rig_with(4, 1);
    r.z.set_chunks(1);
    queue_init(&mut r);
    let s = oom_stream(&mut r, 3);
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!((word(&s), r.z.chunks()), (1, 3));
    let c = u64::from(HEAP_CHUNK_SIZE);
    let link = |i: u64| chunk_link(HEAP_CHUNK_VA + i * c, HEAP_CHUNK_SIZE);
    assert_eq!(r.z.heap_starts(), [link(1), link(2)]);
    let made: Vec<u64> = r.osal.heap_chunks().iter().map(|a| a.gpu).collect();
    assert_eq!(made, [HEAP_CHUNK_VA, HEAP_CHUNK_VA + c, HEAP_CHUNK_VA + 2 * c]);
    assert_eq!((r.osal.oom_chunks, r.osal.oom_no_chunk), (2, 0));
    // the query still answers the heap the backend's context names
    assert_eq!(r.osal.heap().unwrap().first_chunk_gpu, HEAP_CHUNK_VA);
    assert_eq!(r.osal.dead(), None);
}

/// An OOM pending when the next kick is submitted is handled by the submit
/// before it writes the ring; `completed` handles one too.
#[test]
fn a_submit_and_completed_handle_a_pending_tiler_oom() {
    let mut r = rig_with(3, 1);
    r.z.set_chunks(1);
    queue_init(&mut r);
    let a = oom_stream(&mut r, 2);
    let b = oom_stream(&mut r, 3);
    let fa = r.osal.submit(&*r.z, a.gpu, 32, &mut no_delay);
    // time passes with nobody looking: the job runs into the OOM
    for _ in 0..32 {
        r.z.read32(B.gpu);
    }
    assert_eq!(r.z.heap_starts(), []);
    let fb = r.osal.submit(&*r.z, b.gpu, 32, &mut no_delay);
    assert_eq!(fb, fa + 1);
    assert_eq!(r.z.heap_starts().len(), 1, "the submit answered the pending OOM");
    while r.osal.completed(&*r.z, &mut no_delay) < fb {}
    assert_eq!((word(&a), word(&b), r.z.chunks()), (1, 1, 3));
    assert_eq!(r.osal.oom_chunks, 2);
}

/// An OOM the firmware raises at any moment around a submit, also between
/// the OSAL's own look for one and the job's write (the review's probe:
/// N reads of nothing between the two kicks), is serviced: the heap grows,
/// no group restarts, both kicks complete.
#[test]
fn a_tiler_oom_raised_during_a_submit_is_serviced_not_a_restart() {
    for n in 0..40 {
        let mut r = rig_with(3, 1);
        r.z.set_chunks(1);
        queue_init(&mut r);
        let a = oom_stream(&mut r, 2);
        let b = stream(&mut r);
        let fa = r.osal.submit(&*r.z, a.gpu, 32, &mut no_delay);
        for _ in 0..n {
            r.z.read32(B.gpu);
        }
        let fb = r.osal.submit(&*r.z, b.gpu, 24, &mut no_delay);
        assert_eq!(fb, fa + 1, "N = {n}");
        assert_eq!(r.osal.wait(&*r.z, fb, -1, &mut no_delay), 0, "N = {n}");
        assert_eq!(r.osal.wait(&*r.z, fa, -1, &mut no_delay), 0, "N = {n}");
        assert_eq!((r.osal.restarts, r.osal.last_error()), (0, None), "N = {n}");
        assert_eq!((word(&a), word(&b), r.osal.oom_chunks), (1, 1, 1), "N = {n}");
    }
}

/// With every slot the heap's, an OOM is answered with no chunk (panthor's
/// `-ENOMEM`: the firmware waits): acknowledged with `heap_start` 0,
/// counted, the job left waiting, the OSAL still up.
#[test]
fn a_tiler_oom_with_no_slot_left_is_answered_with_no_chunk() {
    let mut r = rig_with(2, 1);
    r.z.set_chunks(1);
    queue_init(&mut r);
    let s = oom_stream(&mut r, 3);
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, 1000, &mut no_delay), 1);
    let link = chunk_link(HEAP_CHUNK_VA + u64::from(HEAP_CHUNK_SIZE), HEAP_CHUNK_SIZE);
    assert_eq!(r.z.heap_starts(), [link, 0]);
    assert_eq!((r.osal.oom_chunks, r.osal.oom_no_chunk), (1, 1));
    assert_eq!((word(&s), r.osal.heap_chunks().len()), (0, 2));
    assert_eq!(r.osal.dead(), None);
}

/// Every error the OSAL restarts after, at the second `CALL` (the first is
/// the queue-init's): two kicks in flight, a buffer freed while the second
/// is recorded. The wait sees the error: both fences are lost (`-EIO`, also
/// when waited again) and complete, the free goes back, the group is
/// restarted with the queue-init run again (the model forgot the heap
/// context at the new start), and the next kick runs on the new ring as
/// the next fence.
#[test]
fn after_each_error_the_group_restarts_and_the_next_kick_runs() {
    let causes = [
        (Fail::Fault(0x0000_0f4b), (SchedRefusal::CsFault, 0x0000_0f4b), 0),
        (Fail::Fatal(0x0000_0044), (SchedRefusal::CsFatal, 0x0000_0044), 0),
        (Fail::Progress, (SchedRefusal::CsgProgressTimeout, 0x8000_0011 | csf::CSG_SYNC_UPDATE), 0),
        // the job ran, and the one after it, before the status was seen
        (Fail::SyncError(0x41), (SchedRefusal::SyncObjError, 0x41), 2),
        // panthor's 5 s, then its own stop and terminate
        (Fail::Hang, (SchedRefusal::JobTimeout, 1), 0),
    ];
    for (fail, why, ran) in causes {
        let mut r = rig();
        queue_init(&mut r);
        let s = stream(&mut r);
        r.z.fail_at(2, fail);
        let f1 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
        let held = r.osal.alloc(4096).unwrap();
        assert_eq!(r.osal.free(&*r.z, held.gpu, held.handle, &mut no_delay), 0);
        let f2 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
        assert_eq!((f1, f2), (2, 3), "{fail:?}");
        assert_eq!(r.osal.wait(&*r.z, f2, -1, &mut no_delay), -EIO, "{fail:?}");
        assert_eq!(r.osal.last_error().map(|(w, _)| w), Some(why.0), "{fail:?}");
        if why.0 != SchedRefusal::CsgProgressTimeout {
            assert_eq!(r.osal.last_error(), Some(why), "{fail:?}");
        }
        assert_eq!((r.osal.restarts, r.osal.dead()), (1, None), "{fail:?}");
        assert_eq!(r.osal.fences(), (3, 3), "{fail:?}");
        assert_eq!(r.osal.wait(&*r.z, f1, -1, &mut no_delay), -EIO, "{fail:?}: a lost fence");
        assert_eq!(r.osal.wait(&*r.z, f2, 0, &mut no_delay), -EIO, "{fail:?}: a lost fence");
        assert_eq!(r.osal.wait(&*r.z, 1, -1, &mut no_delay), 0, "{fail:?}: the queue-init's fence");
        assert_eq!(r.osal.held(), 0, "{fail:?}");
        assert_eq!(r.z.heap_ctx(), Some(HEAP_POOL_VA), "{fail:?}: the queue-init ran again");
        assert_eq!(word(&s), ran, "{fail:?}");

        let f3 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
        assert_eq!(f3, 4, "{fail:?}");
        assert_eq!(r.osal.wait(&*r.z, f3, -1, &mut no_delay), 0, "{fail:?}");
        assert_eq!((r.osal.completed(&*r.z, &mut no_delay), word(&s)), (4, ran + 1), "{fail:?}");
        // the new ring: the queue-init's job, then the kick
        let (insert, extract, seqno) = ring_words(&r);
        assert_eq!((insert, seqno, r.z.stream_extract()), (256, 2, 256), "{fail:?}");
        assert!(extract <= insert, "{fail:?}: {extract}, published late");
        assert_eq!(r.osal.dead(), None, "{fail:?}");
    }
}

/// An OOM the heap refuses (no `HEAP_SET` yet: the context the event names
/// is no heap of the pool) is fatal for the queue, as in panthor: the
/// group restarts, the queue-init names the heap, and the same job then
/// grows the heap and completes.
#[test]
fn an_oom_the_heap_refuses_restarts_the_group_and_the_heap_then_grows() {
    let mut r = rig_with(2, 1);
    r.z.set_chunks(1);
    let s = oom_stream(&mut r, 2);
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.last_error(), Some((SchedRefusal::HeapGrow, 14)));
    assert_eq!((r.osal.restarts, r.osal.dead(), r.z.heap_starts()), (1, None, vec![]));
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!((word(&s), r.osal.heap_chunks().len(), r.osal.oom_chunks), (1, 2, 1));
}

/// The heap is made again at a restart: the chunks grown before go back
/// to their slots and the heap starts again with its initial chunks, the
/// first one where the backend's tiler context names it.
#[test]
fn a_restart_makes_the_heap_again_as_it_started() {
    let mut r = rig_with(3, 1);
    r.z.set_chunks(1);
    queue_init(&mut r);
    let s = oom_stream(&mut r, 3);
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!(r.osal.heap_chunks().len(), 3);
    r.z.fail_at(r.z.calls() + 1, Fail::Fault(1));
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    let made: Vec<u64> = r.osal.heap_chunks().iter().map(|a| a.gpu).collect();
    assert_eq!(made, [HEAP_CHUNK_VA]);
    assert_eq!(r.osal.heap().unwrap().first_chunk_gpu, HEAP_CHUNK_VA);
    // the model's heap starts again too: the job grows it again
    r.z.set_chunks(1);
    let f = r.osal.submit(&*r.z, s.gpu, 32, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    assert_eq!((r.osal.heap_chunks().len(), r.osal.oom_chunks, word(&s)), (3, 4, 2));
}

/// An error seen by a submit (a fault pending when the next kick comes)
/// restarts the group, and the kick goes on the new ring; one seen by
/// `completed` restarts it too and completes every fence.
#[test]
fn a_submit_or_completed_that_meets_an_error_restarts_the_group() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Fault(7));
    let f1 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    for _ in 0..32 {
        r.z.read32(B.gpu);
    }
    let f2 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!((f1, f2, r.osal.restarts), (2, 3, 1));
    assert_eq!(r.osal.wait(&*r.z, f1, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.wait(&*r.z, f2, -1, &mut no_delay), 0);
    assert_eq!(word(&s), 1);

    r.z.fail_at(r.z.calls() + 1, Fail::Fatal(0x44));
    let f3 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    for _ in 0..32 {
        r.z.read32(B.gpu);
    }
    assert_eq!(r.osal.completed(&*r.z, &mut no_delay), f3);
    assert_eq!((r.osal.restarts, r.osal.last_error()), (2, Some((SchedRefusal::CsFatal, 0x44))));
    assert_eq!(r.osal.wait(&*r.z, f3, -1, &mut no_delay), -EIO);
    let f4 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f4, -1, &mut no_delay), 0);
    assert_eq!(word(&s), 2);
}

/// A `CS_FATAL` of type `CS_UNRECOVERABLE` needs panthor's GPU reset, which
/// the OSAL does not do: it is dead, every lost fence complete (so the
/// backend frees its slots), every later kick `-EIO`.
#[test]
fn an_unrecoverable_fatal_leaves_the_osal_dead_with_its_fences_complete() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Fatal(0x0000_0141));
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.dead(), Some((SchedRefusal::CsFatal, 0x0000_0141)));
    assert_eq!((r.osal.restarts, r.osal.completed(&*r.z, &mut no_delay)), (0, f));
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), -i64::from(EIO));
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
}

/// After a restart's terminate is acknowledged and before any memory of
/// the lost kicks is used again (the heap made again, the held frees given
/// back), the GPU's L2 and LSC are cleaned and invalidated (Z2's command,
/// `GPU_CMD` 0x3304): a lost job never reached its block's `FLUSH_CACHE2`.
/// A shutdown does the same after its terminate.
#[test]
fn a_restart_cleans_and_invalidates_the_gpu_caches_after_the_end() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    let flush = (B.gpu + crate::gpu::mali::GPU_CMD, crate::gpu::mali::CMD_FLUSH_L2_LSC);
    let (group, stream_bell) = (B.gpu + csf::doorbell(0), B.gpu + csf::doorbell(1));
    let flushes = |r: &Rig| r.z.writes().iter().filter(|w| **w == flush).count();
    let before = flushes(&r);
    r.z.fail_at(2, Fail::Fault(1));
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    let kicked = r.z.writes().len();
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!((r.osal.restarts, flushes(&r)), (1, before + 1));
    // the group's register writes from the failed kick on: the stop's and
    // the terminate's doorbells, the flush, then the start's doorbell and
    // the queue-init job's
    let after: Vec<usize> = r.z.writes()[kicked..]
        .iter()
        .filter_map(|w| match *w {
            w if w == flush => Some(0),
            (a, _) if a == group => Some(1),
            (a, _) if a == stream_bell => Some(2),
            _ => None,
        })
        .collect();
    let at = after.iter().position(|&w| w == 0).unwrap();
    assert!(after[..at].iter().filter(|&&w| w == 1).count() >= 2, "stop and terminate first: {after:?}");
    assert_eq!((after[at + 1..].contains(&1), after.last()), (true, Some(&2)), "{after:?}");

    let ended = r.z.writes().len();
    assert_eq!(r.osal.shutdown(&*r.z, &mut no_delay).unwrap().code(), 0);
    assert_eq!(flushes(&r), before + 2);
    let log = &r.z.writes()[ended..];
    let bells: Vec<usize> = log.iter().enumerate().filter(|(_, w)| w.0 == group).map(|(i, _)| i).collect();
    let at = log.iter().position(|w| *w == flush).unwrap();
    assert!(bells.len() >= 2 && bells.iter().all(|&i| i < at), "after the stop and the terminate: {bells:?}, {at}");
}

/// A shutdown whose cache clean after the end is refused: the group ended,
/// but the memory is kept, and the answer is neither the clean end's 0
/// nor a refused end's `-EIO`.
#[test]
fn a_shutdown_whose_cache_clean_is_refused_says_so() {
    let mut r = rig();
    let s = stream(&mut r);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    let a = r.osal.alloc(4096).unwrap();
    r.z.flush_stuck.set(true);
    let e = r.osal.shutdown(&*r.z, &mut no_delay).unwrap();
    assert!(e.ended.stop.is_ok() && e.ended.terminate.is_ok(), "{e:?}");
    assert_eq!((e.cleaned, e.code()), (false, -ETIMEDOUT));
    assert_eq!(r.osal.dead().map(|d| d.0), Some(SchedRefusal::CacheFlush));
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    freed_pages_stay_held(&mut r, &a);
}

/// A restart whose queue-init job faults: the new group is stopped and
/// terminated as panthor ends a group with a fatal queue (it never runs
/// again; the next tick terminates it), the OSAL is dead with the fault,
/// and the group is known ended.
#[test]
fn a_restart_whose_queue_init_faults_ends_the_new_group() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Fault(1));
    // the failed CALL is not counted: the queue-init's CALL is the 2nd too
    r.z.fail_at(2, Fail::Fault(2));
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!((r.osal.restarts, r.osal.dead()), (0, Some((SchedRefusal::CsFault, 2))));
    assert_eq!((csg_state(&r), r.osal.unended()), (csf::CSG_STATE_TERMINATE, false));
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), -i64::from(EIO));
}

/// A restart whose queue-init job hangs, and whose terminate after the
/// 5 s is not acknowledged: the group may still run.
#[test]
fn a_restart_whose_queue_init_hangs_unended_is_known() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Fault(1));
    r.z.fail_at(2, Fail::Hang);
    // the restart's own terminate is acknowledged, the init timeout's not
    r.z.terminates_left.set(Some(1));
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.dead().map(|d| d.0), Some(SchedRefusal::JobTimeout));
    assert!(r.osal.unended(), "the init's terminate was not acknowledged");
    assert_eq!(r.osal.alloc(4096), Err(-EIO));
}

/// A restart whose start the firmware does not acknowledge: panthor resets
/// the GPU (panthor_sched.c:2297-2301); the group may start later, so it
/// is not known ended.
#[test]
fn a_restart_whose_start_is_not_acknowledged_is_not_known_ended() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Fault(1));
    r.z.deaf_to_start.set(true);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.dead().map(|d| d.0), Some(SchedRefusal::CsgStartAckTimeout));
    assert!(r.osal.unended());
}

/// A kick that completed before an error, though no call had seen it yet,
/// is not lost: the restart reads the sync object before it marks the
/// kicks not complete as lost.
#[test]
fn a_kick_done_before_the_error_is_not_counted_lost() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(3, Fail::Fault(1));
    let f1 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    // time passes with nobody looking: the kick runs and completes
    for _ in 0..64 {
        r.z.read32(B.gpu);
    }
    assert_eq!((word(&s), r.osal.fences()), (1, (2, 1)), "done, not yet seen");
    let f2 = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f2, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.restarts, 1);
    assert_eq!(r.osal.wait(&*r.z, f1, -1, &mut no_delay), 0, "it completed before the error");
    assert_eq!(r.osal.wait(&*r.z, f2, -1, &mut no_delay), -EIO);
}

/// A buffer freed while kicks are in flight, and a group whose end is not
/// known: the free stays held and no page is handed out again, since the
/// GPU may still run the group and write them.
fn freed_pages_stay_held(r: &mut Rig, f: &Alloc) {
    assert_eq!(r.osal.held(), 1, "the free is still held");
    assert_eq!(r.osal.alloc(4096), Err(-EIO), "no page handed out");
    assert_eq!(r.osal.free(&*r.z, f.gpu, f.handle, &mut no_delay), -EINVAL, "freed once already");
    assert_eq!(r.osal.held(), 1);
}

/// The unrecoverable fatal leaves the group unended (no terminate is
/// sent): what was freed during the lost kick stays held.
#[test]
fn an_unrecoverable_fatal_keeps_the_frees_of_its_kicks_held() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Fatal(0x0000_0141));
    // freed while the kick is recorded: held for its fence
    let a = r.osal.alloc(4096).unwrap();
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.dead().map(|d| d.0), Some(SchedRefusal::CsFatal));
    freed_pages_stay_held(&mut r, &a);
}

/// A hung kick whose stop and terminate the firmware never acknowledges:
/// the OSAL is dead and the group may still run; the frees stay held.
#[test]
fn a_terminate_not_acknowledged_keeps_the_frees_held() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Hang);
    let a = r.osal.alloc(4096).unwrap();
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    r.z.deaf.set(true);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    assert_eq!(r.osal.dead().map(|d| d.0), Some(SchedRefusal::CsgTerminateAckTimeout));
    assert_eq!(r.osal.fences(), (2, 2), "the lost fence is complete");
    freed_pages_stay_held(&mut r, &a);
}

/// A shutdown whose terminate is not acknowledged: a free after it is
/// held, not given back at once as after an acknowledged end.
#[test]
fn a_shutdown_not_acknowledged_keeps_the_frees_held() {
    let mut r = rig();
    let s = stream(&mut r);
    let a = r.osal.alloc(4096).unwrap();
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
    r.z.deaf.set(true);
    let e = r.osal.shutdown(&*r.z, &mut no_delay).unwrap();
    assert!(e.ended.terminate.is_err(), "{e:?}");
    assert_eq!((e.cleaned, e.code()), (false, -EIO));
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    freed_pages_stay_held(&mut r, &a);
}

/// [`RESTART_BUDGET`] restarts, a good kick after each: every lost fence
/// stays `-EIO`, every good one 0. The next error ends the group (stopped
/// and terminated, so the held frees go back) and the OSAL is dead
/// ([`SchedRefusal::RestartBudget`] with the refusal that would have
/// restarted it); every later kick is `-EIO`.
#[test]
fn errors_past_the_restart_budget_end_the_group_and_leave_the_osal_dead() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    let mut lost = vec![];
    let mut good = vec![];
    for _ in 0..RESTART_BUDGET {
        r.z.fail_at(r.z.calls() + 1, Fail::Fault(1));
        let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
        assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
        lost.push(f);
        let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
        assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0);
        good.push(f);
    }
    assert_eq!((r.osal.restarts, word(&s), r.osal.dead()), (RESTART_BUDGET, u64::from(RESTART_BUDGET), None));

    // one more: the group ends, nothing starts again
    r.z.fail_at(r.z.calls() + 1, Fail::Fault(1));
    let a = r.osal.alloc(4096).unwrap();
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
    lost.push(f);
    let budget = (SchedRefusal::RestartBudget, SchedRefusal::CsFault as u32);
    assert_eq!((r.osal.restarts, r.osal.dead(), r.osal.unended()), (RESTART_BUDGET, Some(budget), false));
    assert_eq!(r.osal.last_error(), Some((SchedRefusal::CsFault, 1)));
    assert_eq!(csg_state(&r), csf::CSG_STATE_TERMINATE);
    assert_eq!(r.osal.held(), 0, "the group ended: the free went back");
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), -i64::from(EIO));
    for f in lost {
        assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO, "lost {f}");
    }
    for f in good {
        assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0, "good {f}");
    }
}

/// The group's state field in CSG 0's `ack`, as the model holds it.
fn csg_state(r: &Rig) -> u32 {
    let u = r.osal.up.unwrap();
    r.z.read32(u.csg.output + core::mem::offset_of!(csf::CsgOutput, ack)) & csf::CSG_STATE_MASK
}

/// `shutdown` waits for the kick in flight, then stops CS 0 and terminates
/// CSG 0; later kicks and waits are `-EIO`, a free gives its pages back at
/// once, and a second shutdown has nothing to end.
#[test]
fn shutdown_waits_for_the_last_kick_and_terminates_the_group() {
    let mut r = rig();
    let s = stream(&mut r);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    let a = r.osal.alloc(4096).unwrap();
    assert_eq!(csg_state(&r), csf::CSG_STATE_START);
    let e = r.osal.shutdown(&*r.z, &mut no_delay).unwrap();
    assert!(e.ended.stop.is_ok() && e.ended.terminate.is_ok() && e.cleaned, "{e:?}");
    assert_eq!(e.code(), 0);
    assert_eq!(csg_state(&r), csf::CSG_STATE_TERMINATE);
    assert_eq!((word(&s), r.osal.fences(), r.osal.is_down()), (1, (1, 1), true));
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), 0, "it completed before the end");
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), -i64::from(EIO));
    assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    assert_eq!((r.osal.held(), r.osal.live_pages), (0, 1));
    assert_eq!(r.osal.shutdown(&*r.z, &mut no_delay), None);
    assert_eq!(r.osal.dead(), None);
}

/// A kick that hangs at shutdown: panthor's 5 s, the restart any wait's
/// timeout makes, then the end; its fence is lost.
#[test]
fn shutdown_ends_the_group_after_a_hung_kick_too() {
    let mut r = rig();
    queue_init(&mut r);
    let s = stream(&mut r);
    r.z.fail_at(2, Fail::Hang);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    let e = r.osal.shutdown(&*r.z, &mut no_delay).unwrap();
    assert!(e.ended.stop.is_ok() && e.ended.terminate.is_ok() && e.cleaned, "{e:?}");
    assert_eq!(e.code(), 0);
    assert_eq!((r.osal.restarts, r.osal.last_error().unwrap().0), (1, SchedRefusal::JobTimeout));
    assert_eq!(csg_state(&r), csf::CSG_STATE_TERMINATE);
    assert_eq!(r.osal.wait(&*r.z, f, -1, &mut no_delay), -EIO);
}

/// A full pending-free queue (src/osal/pending_frees.h's rule): a free
/// held for the kick being recorded, which no wait can complete, is
/// `-EBUSY`; once that kick is submitted, the next free waits for it and is
/// held.
#[test]
fn a_full_free_queue_waits_for_its_oldest_kick() {
    let mut r = rig();
    let s = stream(&mut r);
    let pages: Vec<Alloc> = (0..=PENDING_FREES).map(|_| r.osal.alloc(4096).unwrap()).collect();
    for a in &pages[..PENDING_FREES] {
        assert_eq!(r.osal.free(&*r.z, a.gpu, a.handle, &mut no_delay), 0);
    }
    let last = pages[PENDING_FREES];
    assert_eq!(r.osal.free(&*r.z, last.gpu, last.handle, &mut no_delay), -EBUSY);
    assert_eq!((r.osal.held(), r.osal.refused_frees), (PENDING_FREES, 1));
    // the kick they were held for is submitted, not yet complete
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    let b = r.osal.alloc(4096).unwrap();
    assert_eq!(r.osal.fences(), (1, 0));
    assert_eq!(r.osal.free(&*r.z, b.gpu, b.handle, &mut no_delay), 0);
    assert_eq!(r.osal.fences(), (f as u64, 1), "the free waited for the kick");
    assert_eq!((r.osal.held(), r.osal.refused_frees), (1, 1));
    // the stream's page, the refused free's page kept, the last free held
    assert_eq!(r.osal.live_pages, 3);
}

/// A wait shorter than the job takes returns 1 and leaves the job running;
/// the next wait sees it complete.
#[test]
fn a_short_wait_times_out_and_leaves_the_job_running() {
    let mut r = rig();
    let s = stream(&mut r);
    let f = r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay);
    assert_eq!(r.osal.wait(&*r.z, f, 0, &mut no_delay), 1);
    assert_eq!(r.osal.completed(&*r.z, &mut no_delay), 0);
    assert_eq!(r.osal.dead(), None);
    assert_eq!(r.osal.wait(&*r.z, f, 1000, &mut no_delay), 0);
    assert_eq!((r.osal.completed(&*r.z, &mut no_delay), word(&s)), (1, 1));
}

/// A stream outside the carve-out, misaligned or of no whole words is
/// refused before the ring is touched; the OSAL goes on.
#[test]
fn a_kick_the_carve_out_does_not_hold_is_refused() {
    let mut r = rig();
    let s = stream(&mut r);
    let e = -i64::from(EINVAL);
    assert_eq!(r.osal.submit(&*r.z, HEAP_CHUNK_VA, 24, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu + 8, 24, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 0, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 20, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, CARVE_VA + CARVE - 64, 128, &mut no_delay), e);
    assert_eq!(r.osal.submit(&*r.z, s.gpu, 24, &mut no_delay), 1);
}

/// `map_phys` of the carve-out's own pages answers them where they are.
#[test]
fn a_window_is_the_carve_out_s_own_pages() {
    let mut r = rig();
    let a = r.osal.alloc(3 * 4096).unwrap();
    let w = r.osal.window(a.phys + 0x100, 0x2000).unwrap();
    assert_eq!((w.gpu, w.cpu, w.handle), (a.gpu + 0x100, a.cpu + 0x100, 0));
    assert_eq!(r.osal.window(a.phys - 1, 16), Err(-ENOTSUP));
    assert_eq!(r.osal.window(a.phys + CARVE - 8, 16), Err(-ENOTSUP));
    assert_eq!(r.osal.window(a.phys, 0), Err(-EINVAL));
}

/// The sections one after the other from page boundaries, in the image's
/// order; an arena too small for them is no placement.
#[test]
fn the_sections_are_placed_page_by_page_in_the_arena() {
    let blob = synth::small();
    let img = fw::parse(&blob).unwrap();
    let arena = Backing { pa: 0x2000_0000, cpu: 0x7000_0000, size: 1 << 20 };
    let (b, n) = place_sections(&blob, arena).unwrap();
    assert_eq!(n, img.sections().len());
    let mut at = 0;
    for (b, s) in b[..n].iter().zip(img.sections()) {
        assert_eq!((b.pa, b.cpu, b.size), (arena.pa + at, arena.cpu + at as usize, u64::from(s.va_size())));
        at += u64::from(s.va_size()).next_multiple_of(PAGE);
    }
    assert!(place_sections(&blob, Backing { size: at - 1, ..arena }).is_none());
    assert!(place_sections(b"not an image", arena).is_none());
}

mod gl {
    //! The L2 triangle (driver/docs/nanopc-t6-mali.md; panthor-gl's
    //! default program): EGL, a 64x64 pbuffer and an ES 2 context, an FBO
    //! on a 64x64 RGBA8 texture, cleared blue, tri.vert / tri.frag drawn
    //! over the left half, glReadPixels. The library's ops are the OSAL's
    //! methods over the model, as zephyr_mali.c's are the same methods
    //! through the C entries.

    use super::*;
    use core::ffi::c_void;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use yakogl::gl::consts::*;
    use yakogl::gl::draw::glDrawArrays;
    use yakogl::gl::egl::{
        eglChooseConfig, eglCreateContext, eglCreatePbufferSurface, eglGetDisplay, eglInitialize, eglMakeCurrent,
    };
    use yakogl::gl::egl_consts::*;
    use yakogl::gl::framebuffer::glReadPixels;
    use yakogl::gl::framebuffer::{
        glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers,
    };
    use yakogl::gl::program::{glAttachShader, glBindAttribLocation, glCreateProgram, glLinkProgram, glUseProgram};
    use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
    use yakogl::gl::state::{glClear, glClearColor, glFinish, glGetError, glViewport};
    use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
    use yakogl::osal::mali::{OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
    use yakogl::osal::{OsalGpuMem, OsalOps};

    const TRI_VS: &str = include_str!("../../../../../tools/mali/testdata/tri.vert");
    const TRI_FS: &str = include_str!("../../../../../tools/mali/testdata/tri.frag");
    /// Two triangles over the left half (tools/mali/panthor/src/l1.rs's
    /// `VERTICES`).
    const VERTICES: [f32; 12] = [-1.0, -1.0, 0.0, -1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, -1.0, 1.0];

    /// The rig and every allocation's (GPU VA, bytes), in order.
    struct Gl {
        r: Rig,
        allocs: Vec<(u64, usize, usize)>,
    }

    static GL: AtomicPtr<Gl> = AtomicPtr::new(core::ptr::null_mut());

    fn with<R>(f: impl FnOnce(&mut Gl) -> R) -> R {
        let p = GL.load(Ordering::Acquire);
        assert!(!p.is_null(), "the rig is installed");
        // SAFETY: the test's Gl, alive while installed; the library calls
        // the ops from the test's thread only, one at a time
        f(unsafe { &mut *p })
    }

    fn fill(out: *mut OsalGpuMem, a: &Alloc) {
        // SAFETY: the library's out-parameter
        unsafe {
            *out = OsalGpuMem {
                cpu: a.cpu as *mut c_void,
                gpu: a.gpu,
                phys: a.phys,
                size: a.size,
                impl_: a.handle as *mut c_void,
            }
        };
    }

    unsafe extern "C" fn o_init(_: *mut c_void) -> i32 {
        use yakogl::shaderc::online::store;
        static LENT: std::sync::Once = std::sync::Once::new();
        LENT.call_once(|| {
            let scratch: &'static mut [u8] = Box::leak(vec![0u8; store::SCRATCH_BYTES].into_boxed_slice());
            let keep: &'static mut [u64] = Box::leak(vec![0u64; store::KEEP_BYTES / 8].into_boxed_slice());
            // SAFETY: both leaked, so they outlive every compile
            let rc = unsafe {
                (
                    store::shc_online_lend_scratch(scratch.as_mut_ptr(), scratch.len()),
                    store::shc_online_lend_keep(keep.as_mut_ptr().cast(), store::KEEP_BYTES),
                )
            };
            assert_eq!(rc, (0, 0));
        });
        0
    }

    unsafe extern "C" fn o_alloc(_: u64, size: usize, _: u32, out: *mut OsalGpuMem) -> i32 {
        with(|g| match g.r.osal.alloc(size) {
            Ok(a) => {
                g.allocs.push((a.gpu, size, a.cpu));
                fill(out, &a);
                0
            }
            Err(e) => e,
        })
    }

    unsafe extern "C" fn o_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the library's descriptor
        let m = unsafe { &mut *mem };
        if m.impl_.is_null() {
            return 0;
        }
        let rc = with(|g| g.r.osal.free(&*g.r.z, m.gpu, m.impl_ as usize, &mut no_delay));
        *m = OsalGpuMem::ZERO;
        rc
    }

    unsafe extern "C" fn o_map_phys(_: u64, phys: u64, size: usize, _: u32, out: *mut OsalGpuMem) -> i32 {
        with(|g| match g.r.osal.window(phys, size) {
            Ok(a) => {
                fill(out, &Alloc { handle: 0, ..a });
                0
            }
            Err(e) => e,
        })
    }

    unsafe extern "C" fn o_cache(_: *mut c_void, _: usize) {}

    unsafe extern "C" fn o_submit(kick: *const c_void, len: usize) -> i64 {
        if kick.is_null() || len != size_of::<OsalMaliKick>() {
            return -i64::from(EINVAL);
        }
        // SAFETY: the backend's kick of the size checked
        let k = unsafe { *kick.cast::<OsalMaliKick>() };
        with(|g| g.r.osal.submit(&*g.r.z, k.stream_gpu, k.stream_bytes, &mut no_delay))
    }

    unsafe extern "C" fn o_wait(fence: i64, timeout_us: i64) -> i32 {
        with(|g| g.r.osal.wait(&*g.r.z, fence, timeout_us, &mut no_delay))
    }

    unsafe extern "C" fn o_completed() -> i64 {
        with(|g| g.r.osal.completed(&*g.r.z, &mut no_delay))
    }

    unsafe extern "C" fn o_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
        with(|g| {
            // SAFETY: the caller's out-parameter of `sz` bytes, checked
            // against each answer's size
            unsafe {
                match what {
                    OSAL_QUERY_MALI_GPU if sz == size_of::<OsalMaliGpu>() => {
                        *out.cast::<OsalMaliGpu>() = g.r.osal.gpu_info().unwrap()
                    }
                    OSAL_QUERY_MALI_TILER_HEAP if sz == size_of::<OsalMaliHeap>() => {
                        *out.cast::<OsalMaliHeap>() = g.r.osal.heap().unwrap()
                    }
                    _ => return -ENOTSUP,
                }
            }
            0
        })
    }

    /// The log hook: the two arguments `osal::log_str` passes (`"%s"`,
    /// line), printed so a failing run shows the library's lines.
    extern "C" fn o_log(_fmt: *const core::ffi::c_char, line: *const core::ffi::c_char) {
        if !line.is_null() {
            // SAFETY: osal::log_str passes a NUL-terminated line
            println!("yakogl: {}", unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy());
        }
    }

    /* `log` is a C variadic function, which stable Rust cannot define: a
     * one-instruction trampoline declared as the variadic function it is
     * tail-jumps to o_log with the argument registers untouched (the
     * leading pointer arguments are passed as in a non-variadic call on
     * x86-64 System V and AArch64 AAPCS64), as tools/mali/panthor's OSAL
     * and the library's own test hook do. */
    #[cfg(target_arch = "x86_64")]
    core::arch::global_asm!(
        ".pushsection .text.yd_osal_test_log,\"ax\",@progbits",
        ".globl yd_osal_test_log",
        ".hidden yd_osal_test_log",
        "yd_osal_test_log:",
        "jmp {log}",
        ".popsection",
        log = sym o_log,
    );
    #[cfg(target_arch = "aarch64")]
    core::arch::global_asm!(
        ".pushsection .text.yd_osal_test_log,\"ax\",%progbits",
        ".globl yd_osal_test_log",
        ".hidden yd_osal_test_log",
        "yd_osal_test_log:",
        "b {log}",
        ".popsection",
        log = sym o_log,
    );
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    extern "C" {
        fn yd_osal_test_log(fmt: *const core::ffi::c_char, ...);
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    const LOG: Option<unsafe extern "C" fn(*const core::ffi::c_char, ...)> = Some(yd_osal_test_log);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const LOG: Option<unsafe extern "C" fn(*const core::ffi::c_char, ...)> = None;

    static OPS: OsalOps = OsalOps {
        init: Some(o_init),
        shutdown: None,
        gpu_alloc: Some(o_alloc),
        gpu_free: Some(o_free),
        mmu_map: None,
        map_phys: Some(o_map_phys),
        gpu_write: None,
        gpu_read: None,
        cache_flush: Some(o_cache),
        cache_invd: Some(o_cache),
        submit: Some(o_submit),
        wait: Some(o_wait),
        completed: Some(o_completed),
        reg_read: None,
        reg_write: None,
        query: Some(o_query),
        log: LOG,
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    };

    /// A compiled shader of `src`.
    fn shader(kind: GLenum, src: &str) -> GLuint {
        // SAFETY: plain GL calls on the test's thread, with a current
        // context; the pointers live across the calls
        unsafe {
            let s = glCreateShader(kind);
            let p = src.as_ptr().cast::<GLchar>();
            let n = src.len() as GLint;
            glShaderSource(s, 1, &p, &n);
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile of {src:?}");
            s
        }
    }

    /// The red and blue pixels where the triangle puts them.
    fn red_blue(px: &[u8]) -> (usize, usize) {
        let words: Vec<u32> = px.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let red = words.iter().enumerate().filter(|&(i, &w)| w == RED && i % 64 < 32).count();
        let blue = words.iter().enumerate().filter(|&(i, &w)| w == BLUE && i % 64 >= 32).count();
        (red, blue)
    }

    /// The L2 program through the OSAL over the model. The model runs each
    /// job the ring holds: the job's `CALL` runs the backend's stream at
    /// its VA in the carve-out, its `SYNC_ADD64` completes the fence, and
    /// after a `CALL` the model paints the texture as the triangle would
    /// (red left half, blue right): the pixels are the model's stand-in, the
    /// path to them is the OSAL's (the board run checks the GPU's own).
    /// Then K2's recovery: the next frame's kick faults, the frame is lost
    /// and counted (`yakogl_gpu_work_lost`), the OSAL restarts its group,
    /// and the frame after it draws again.
    #[test]
    fn the_l2_triangle_draws_through_the_osal_and_again_after_a_lost_frame() {
        let mut g = Box::new(Gl { r: rig(), allocs: vec![] });
        GL.store(&mut *g, Ordering::Release);
        let (lost0, refused0) = (yakogl::backend::yakogl_gpu_work_lost(), yakogl::backend::yakogl_draws_refused());
        // SAFETY: OPS is a static; plain EGL and GL calls on this thread
        unsafe {
            yakogl::osal::osal_set_ops(&OPS);
            let dpy = eglGetDisplay(core::ptr::null_mut());
            assert_ne!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 0);
            let init_kicks = g.r.osal.fences().0;
            assert!(init_kicks >= 1, "the backend's queue-init stream ran");
            let cfg_attrs = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
            let mut cfg: EGLConfig = core::ptr::null_mut();
            let mut n = 0;
            assert!(eglChooseConfig(dpy, cfg_attrs.as_ptr(), &mut cfg, 1, &mut n) != 0 && n == 1);
            let surf = eglCreatePbufferSurface(dpy, cfg, [EGL_WIDTH, 64, EGL_HEIGHT, 64, EGL_NONE].as_ptr());
            let ctx =
                eglCreateContext(dpy, cfg, core::ptr::null_mut(), [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE].as_ptr());
            assert!(!surf.is_null() && !ctx.is_null() && eglMakeCurrent(dpy, surf, surf, ctx) != 0);

            let (mut tex, mut fbo) = (0, 0);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            let before = g.allocs.len();
            let pattern = vec![0x5au8; 64 * 64 * 4];
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                64,
                64,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pattern.as_ptr().cast(),
            );
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            /* the texture's storage: the one buffer glTexImage2D took, level 0
             * first, linear, 256-byte rows (its mip levels' rows after) */
            let texels = &g.allocs[before..];
            assert!(texels.len() == 1 && texels[0].1 >= 64 * 64 * 4, "{texels:x?}");
            g.r.z.paint_at(texels[0].0);

            glViewport(0, 0, 64, 64);
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, TRI_VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, TRI_FS));
            glBindAttribLocation(p, 0, c"pos".as_ptr());
            glLinkProgram(p);
            glUseProgram(p);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, VERTICES.as_ptr().cast());
            glDrawArrays(GL_TRIANGLES, 0, 6);
            let mut px = vec![0u8; 64 * 64 * 4];
            glReadPixels(0, 0, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR);

            assert_eq!(red_blue(&px), (2048, 2048));
            /* the frame went through the ring: one job a kick, each kick's
             * CALL run, every fence complete, nothing lost or refused */
            let (sub, done) = g.r.osal.fences();
            assert!(sub > init_kicks, "the frame was a kick of its own");
            assert_eq!((done, u64::from(g.r.z.calls())), (sub, sub));
            assert_eq!(g.r.osal.dead(), None);
            assert_eq!(yakogl::backend::yakogl_gpu_work_lost(), lost0);
            assert_eq!(yakogl::backend::yakogl_draws_refused(), refused0);

            /* the texture back to its fill, so that only a frame that runs
             * paints it, then a frame whose kick faults */
            let target = texels[0];
            let fill = || core::ptr::write_bytes(target.2 as *mut u8, 0x5a, 64 * 64 * 4);
            let draw = |px: &mut [u8]| {
                glClear(GL_COLOR_BUFFER_BIT);
                glDrawArrays(GL_TRIANGLES, 0, 6);
                glReadPixels(0, 0, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
                glFinish();
            };
            fill();
            g.r.z.fail_at(g.r.z.calls() + 1, Fail::Fault(0x0000_0f4b));
            draw(&mut px);
            assert!(yakogl::backend::yakogl_gpu_work_lost() > lost0, "the lost frame is counted");
            assert_eq!((g.r.osal.restarts, g.r.osal.dead()), (1, None));
            assert_eq!(g.r.osal.last_error(), Some((SchedRefusal::CsFault, 0x0000_0f4b)));
            assert_eq!(g.r.z.heap_ctx(), Some(HEAP_POOL_VA), "the restart ran the queue-init again");
            let (sub, done) = g.r.osal.fences();
            assert_eq!(done, sub, "every fence complete: the backend's slots are free");
            /* the next frame draws, on the new ring */
            let lost1 = yakogl::backend::yakogl_gpu_work_lost();
            fill();
            let mut px = vec![0u8; 64 * 64 * 4];
            draw(&mut px);
            assert_eq!(red_blue(&px), (2048, 2048));
            assert_eq!(yakogl::backend::yakogl_gpu_work_lost(), lost1);
            assert_eq!(yakogl::backend::yakogl_draws_refused(), refused0);
            let (sub2, done2) = g.r.osal.fences();
            assert!(sub2 > sub && done2 == sub2, "the frame was a kick, complete: {sub2} {done2}");
            assert_eq!(g.r.osal.restarts, 1);
            yakogl::osal::osal_set_ops(core::ptr::null());
        }
        GL.store(core::ptr::null_mut(), Ordering::Release);
    }
}
