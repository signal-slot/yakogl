// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The two buffers the back end lends the compiler.
//!
//! [`arena`](super::arena) is a bump allocator over a buffer *the caller lends
//! it*, which is the right shape for a front end: a compile is short-lived and
//! everything it builds dies with the call. The back end has to supply those
//! buffers, and it needs two of them, because a compiled shader outlives its
//! compile:
//!
//! | | Reset | Holds |
//! |---|---|---|
//! | [`with_scratch`] | every compile | tokens, AST, IR, SSA, the register allocation |
//! | [`keep`] | per shader, when its last holder lets go | the USC dwords, the `ShcUniform` rows, the name pool, the source |
//!
//! `keep` outlives the compile because the `ShcShader` the GL core copied
//! points into it and the C ABI has nowhere to put a lifetime. It is not a
//! bump arena over the whole region: each compile gets a bump arena over the
//! largest free run ([`begin_keep_at`]), at the run's bottom or top by the
//! record's lifetime ([`KeepEnd`]: a shader object's record at the top, a
//! program's at the bottom, so that one freed does not strand a hole below
//! the other), and what it allocated becomes one [`KeepExtent`] its shader
//! owns. `backend.rs` counts the holders of each
//! shader (the shader object, each program that linked it, each blend
//! variant) and gives the extent back with [`free_keep`] when the count
//! reaches zero -- so `glDeleteShader` and `glDeleteProgram` return the
//! memory, and a process that compiles for ever runs in a bounded region.
//!
//! Both are `.bss` regions of this library, because the product targets are
//! Zephyr and bare metal and the crate has no allocator. The sizes below are
//! **not measured**: nothing has compiled through this path yet. They are set
//! at M4 from the peak of the regression set, which is what [`scratch_peak`]
//! and [`keep_used`] are for; until then a build that needs more says so
//! rather than failing obscurely.

#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
use core::cell::UnsafeCell;
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
use core::mem::MaybeUninit;

use super::arena::Arena;
use crate::util::Global;

/// Scratch bytes per compile: tokens, AST, IR, SSA, register allocation.
///
/// **Measured, and the measurement has moved in both directions**, which is
/// the thing to know before anyone sizes a product constant from it.
/// `tests/scratch_survey.rs` is the instrument; it prints the corpus size and
/// the date beside the number, because two readings are not comparable
/// without both halves.
///
/// | measured | peak | corpus | what changed |
/// |---|---|---|---|
/// | 13:32 | 931,496 | 307 rows | the original figure |
/// | ~16:00 | 447,496 | 324 rows | the front end rewritten a dozen times since |
/// | after if-conversion | 515,472 | 324 rows | more sources reach selection and assembly |
///
/// It fell by half and then rose again within one day. **The front end is 85%
/// of the deepest compile** (439,880 of 515,472 at the last reading, for
/// `fill_image_gradient_conic.glyph.frag`), so this number tracks the front
/// end's work far more than the back end's.
///
/// So it is not a bound on anything: it is the deepest single compile the
/// compiler happened to perform, and it rises when coverage grows and falls
/// when lowering gets leaner. A **larger shader than any in the corpus**
/// exceeds every figure above, and the corpus is not a bound on what an
/// application compiles. At the previous 192 KiB, 38 sources failed with
/// "compiler scratch buffer exhausted" — the arena, not the shader — which is
/// what that failure looks like from the outside.
///
/// **4 MiB since 2026-09-25**: the AM62P's host pre-run of dEQP-GLES2
/// refused two legal shaders at 2 MiB, measured with growing arenas on the
/// PowerVR online back end: light_amount.16omni's vertex shader (sixteen
/// unrolled omni lights) needs 3 MiB (Prepare ran out at 2), and
/// uniform_api.random.23's fragment shader needs 4 MiB (Select ran out at
/// 2, Alloc at 3). Pinned by usc_limits
/// `the_default_scratch_compiles_the_pre_runs_two_largest_shaders`. How much
/// scratch the back end's selection and allocation take per program is
/// the real item, and stays open.
///
/// **Open, and a product question rather than a compiler one**: 4 MiB of
/// `.bss` is a real cost in a Zephyr image. The options are a smaller region
/// with big shaders refused (which this already reports honestly), or letting
/// the caller lend the region as `arena::Arena` already allows. Nobody has
/// decided, and guessing small would turn a measured requirement into
/// intermittent refusals on the target.
pub const SCRATCH_BYTES: usize = 4 * 1024 * 1024;
/// Bytes for everything the compiled shaders keep: code, reflection, names.
///
/// **Re-measured 2026-09-22 after it ran out.** The old value, 128 KiB, was
/// set from a run in which two sources compiled and it used 5,288 bytes --
/// "room by a wide margin today", and the margin lasted about a day. When
/// coverage reached 82 sources the arena filled, and 43 of them were refused
/// with `Abi(OutOfMemory)` -- so the oracle reported **39 of 142 when the
/// truth was 82**, and the missing 43 looked like compiler gaps rather than a
/// constant set too low.
///
/// The measurement now: **300,768 bytes** for the whole 324-source corpus
/// compiled in one process, about 3.7 KiB per compiled shader. 512 KiB leaves
/// roughly 70 % headroom over a corpus far larger than any one application
/// builds, and the harness prints the high-water mark against this constant
/// every run, which is what makes the next exhaustion visible before it is
/// mistaken for a gap.
///
/// Unlike the scratch it is not reset per compile, because the `ShcShader`
/// the GL core holds points into it; each shader's share is given back when
/// the GL core lets go of the last copy of it (see the module note). So what
/// this must hold is the shaders an application keeps ALIVE at once, not
/// every shader it ever compiled -- until 2026-09-23 it was the latter, and a
/// process was refused every compile after about 200.
///
/// And it must hold them in ONE free run per compile, which placement
/// decides ([`KeepEnd`]). femtovg's 30 programs on the V3D hold 388,544
/// bytes, packed, with the counted loop (ea8ab0d6); with both lifetimes at
/// the bottom they ran out at program 22 while 230 KB was free
/// (`v3d_abi::tests::femtovgs_thirty_programs_link_and_stay_alive_together`
/// prints the figures per program, 2026-09-24).
///
/// This is the size of the BUILT-IN region, which a host build uses. An
/// image has none: its OSAL lends the region ([`shc_online_lend_keep`]),
/// sized by `CONFIG_YAKOGL_SHADERC_KEEP_BYTES` (default 512 KiB, the same),
/// so an application that keeps more sets more.
pub const KEEP_BYTES: usize = 512 * 1024;

/* The built-in regions' type: an image has none (see below). */
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
#[repr(align(16))]
struct Region<const N: usize>(UnsafeCell<MaybeUninit<[u8; N]>>);

// SAFETY: YakoGL is single-threaded by contract (see `util::Global`), and each
// region is only reachable through the one entry point below.
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
unsafe impl<const N: usize> Sync for Region<N> {}

/* The crate's own region, for a build that has not been lent one.
 *
 * An image does NOT get this: `no-shader-table` is the image configuration,
 * and there the region comes from the OSAL, sized by
 * CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES. Two megabytes of .bss that the caller
 * then duplicates is the worst of both, so this is compiled out there and a
 * compile before the lend is refused by name rather than served from a
 * region nobody asked for.
 *
 * A host test build keeps it even with `no-shader-table`: `host-test` is
 * never an image (lib.rs refuses it on a bare-metal target, capi refuses to
 * archive it), and without it the compiler's own tests could not run on a
 * build with no table -- which is every `gpu-v3d` build, whose GPU has no
 * table at all. */
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
static SCRATCH_MEM: Region<SCRATCH_BYTES> = Region(UnsafeCell::new(MaybeUninit::uninit()));

/// The region the caller lent, if any: pointer and length.
///
/// Set once by [`shc_online_lend_scratch`] before the first compile. A lent
/// region always wins over the built-in one, so a host build can exercise the
/// lending path without being rebuilt.
static LENT: Global<Option<(*mut u8, usize)>> = Global::new(None);
/* The crate's own keep region, compiled out of an image exactly as the
 * scratch is: there the OSAL lends one sized by
 * CONFIG_YAKOGL_SHADERC_KEEP_BYTES ([`shc_online_lend_keep`]), and a compile
 * before the lend is refused by name. */
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
static KEEP_MEM: Region<KEEP_BYTES> = Region(UnsafeCell::new(MaybeUninit::uninit()));

/// Set while [`with_scratch`] has the scratch region lent out.
static SCRATCH_BUSY: Global<bool> = Global::new(false);
/// The most scratch any compile has used so far.
static SCRATCH_PEAK: Global<usize> = Global::new(0);
/// The high-water of the most recent compile alone.
static LAST_SCRATCH_PEAK: Global<usize> = Global::new(0);
/// The most the keep region has held at once, across resets and frees: the
/// number a product's region has to be sized from.
static KEEP_PEAK: Global<usize> = Global::new(0);

/// The keep region's allocation unit. Every compiled shader owns a whole
/// number of these, so a shader freed out of order leaves a hole the next
/// one can reuse. 64 bytes wastes about 32 per shader on average, against
/// records of 3-6 KiB, and keeps the map at a kilobyte.
pub const KEEP_GRANULE: usize = 64;
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
const KEEP_GRANULES: usize = KEEP_BYTES / KEEP_GRANULE;
const _: () = assert!(KEEP_BYTES.is_multiple_of(KEEP_GRANULE * 64), "the map is whole u64 words");

/// One bit per granule of the built-in keep region: set while a compiled
/// shader owns it. A lent region carries its own map at its head.
#[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
static KEEP_MAP: Global<[u64; KEEP_GRANULES / 64]> = Global::new([0; KEEP_GRANULES / 64]);

/// The keep region in use: its first granule's address, its granule count
/// (a whole number of map words), and its map.
#[derive(Clone, Copy)]
struct KeepRegion {
    base: *mut u8,
    granules: usize,
    map: *mut u64,
}

/// The region the caller lent ([`shc_online_lend_keep`]), if any. It wins
/// over the built-in one, as a lent scratch does.
static KEEP_LENT: Global<Option<KeepRegion>> = Global::new(None);

/// The keep region: the lent one, else this build's own, else none (an
/// image before the lend: zero granules, so every compile is refused).
fn region() -> KeepRegion {
    // SAFETY: single-threaded by contract; written only by the lend.
    if let Some(r) = *unsafe { KEEP_LENT.get() } {
        return r;
    }
    #[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
    {
        KeepRegion {
            base: KEEP_MEM.0.get() as *mut u8,
            granules: KEEP_GRANULES,
            // SAFETY: single-threaded by contract.
            map: unsafe { KEEP_MAP.get() }.as_mut_ptr(),
        }
    }
    #[cfg(all(feature = "no-shader-table", not(feature = "host-test")))]
    {
        KeepRegion {
            base: core::ptr::NonNull::dangling().as_ptr(),
            granules: 0,
            map: core::ptr::NonNull::dangling().as_ptr(),
        }
    }
}

/// Which end of the largest free run a compile's extent is placed at.
///
/// The region holds records of two lifetimes, and placing both at the same
/// end is what fragmented it (femtovg on the Pi 4, 2026-09-24): a V3D stage
/// record lives as long as its shader object, which femtovg deletes right
/// after the link; the program record the link builds lives as long as the
/// program. Both opened at the bottom of the largest run, so each program
/// record landed above its stage record, and each freed stage record left a
/// 10.6 KB hole that the next 13.9 KB program record did not fit: after 21
/// programs the region was 294 KB in use and the largest run 10.6 KB.
///
/// So each lifetime has an end: a record a shader object owns goes to the
/// top of the largest run ([`KeepEnd::High`]), a record a program owns to the
/// bottom ([`KeepEnd::Low`]). A stage record freed after its link then
/// rejoins the free run between the two, instead of leaving a hole below a
/// program. Nothing moves once placed; this only chooses where a record
/// starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeepEnd {
    /// The bottom: records a program owns (a link's product, a blend
    /// variant, a recompile with varyings read as zero).
    Low,
    /// The top: records a shader object owns (glCompileShader's).
    High,
}

/// The keep arena that is open, if any: the free run it is over (first
/// granule and length) and a bump arena over it, upward from the run's
/// start or, for [`KeepEnd::High`], downward from its end.
///
/// Open for the length of one compile ([`begin_keep`]), which is what lets
/// the code generators keep calling [`keep`] as the bump arena they have
/// always had, while what they allocate still becomes ONE extent the shader
/// owns and gives back. `ambient` marks an arena [`keep`] opened by itself,
/// outside any compile -- only tests do that -- and it is kept, never freed,
/// when the next compile opens its own.
struct Open {
    start: usize,
    len: usize,
    end: KeepEnd,
    arena: Arena<'static>,
    ambient: bool,
}

/// Tests only: place every compile at [`KeepEnd::Low`], the one policy the
/// region had before [`KeepEnd`], so that a test can run the old placement
/// and the new one in one process and show that they differ.
#[cfg(test)]
static SINGLE_ENDED: Global<bool> = Global::new(false);

/// Tests only: see [`SINGLE_ENDED`]. Returns the previous setting.
#[cfg(test)]
pub fn set_single_ended(on: bool) -> bool {
    // SAFETY: single-threaded by contract; tests hold `test_lock`.
    core::mem::replace(unsafe { SINGLE_ENDED.get() }, on)
}
static KEEP_OPEN: Global<Option<Open>> = Global::new(None);

/// The granules one compiled shader owns in the keep region, to be given
/// back with [`free_keep`] when the last holder of the shader lets it go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeepExtent {
    start: u32,
    granules: u32,
}

impl KeepExtent {
    /// No granules: what a record carries until its extent is committed.
    pub const EMPTY: KeepExtent = KeepExtent { start: 0, granules: 0 };

    /// The bytes this extent covers.
    pub fn bytes(&self) -> usize {
        self.granules as usize * KEEP_GRANULE
    }
}

fn map() -> &'static mut [u64] {
    let r = region();
    // SAFETY: `r.map` holds `r.granules / 64` words (the built-in map, or the
    // head of the lent region); single-threaded by contract, and callers use
    // it for one operation.
    unsafe { core::slice::from_raw_parts_mut(r.map, r.granules / 64) }
}

fn granule_used(g: usize) -> bool {
    map()[g / 64] & (1u64 << (g % 64)) != 0
}

fn mark(start: usize, n: usize, used: bool) {
    let m = map();
    for g in start..start + n {
        if used {
            m[g / 64] |= 1u64 << (g % 64);
        } else {
            m[g / 64] &= !(1u64 << (g % 64));
        }
    }
}

/// The longest run of free granules: its start and its length.
fn largest_free_run() -> (usize, usize) {
    largest_run_where(granule_used)
}

/// The longest run of granules `used` says are free, in the region.
fn largest_run_where(used: impl Fn(usize) -> bool) -> (usize, usize) {
    largest_run_in(region().granules, used)
}

/// The longest run of free granules among `granules` (start, length); the
/// first of equal ones. A function of its arguments alone, so a test can
/// check it on a map of its own.
fn largest_run_in(granules: usize, used: impl Fn(usize) -> bool) -> (usize, usize) {
    let (mut best, mut best_len) = (0, 0);
    let mut g = 0;
    while g < granules {
        if used(g) {
            g += 1;
            continue;
        }
        let s = g;
        while g < granules && !used(g) {
            g += 1;
        }
        if g - s > best_len {
            (best, best_len) = (s, g - s);
        }
    }
    (best, best_len)
}

/// Open a keep arena over the largest free run, at `end` of it. The caller
/// has checked that none is open.
fn open(ambient: bool, end: KeepEnd) -> &'static Open {
    #[cfg(test)]
    // SAFETY: single-threaded by contract.
    let end = if *unsafe { SINGLE_ENDED.get() } { KeepEnd::Low } else { end };
    let (start, len) = largest_free_run();
    // SAFETY: the granules [start, start + len) are free, so no compiled
    // shader points into them, and this is the only arena over them until it
    // is closed (single-threaded; one open arena at a time).
    let buf = unsafe { core::slice::from_raw_parts_mut(region().base.add(start * KEEP_GRANULE), len * KEEP_GRANULE) };
    let arena = match end {
        KeepEnd::Low => Arena::new(buf),
        KeepEnd::High => Arena::new_top_down(buf),
    };
    // SAFETY: single-threaded by contract.
    let slot = unsafe { KEEP_OPEN.get() };
    slot.insert(Open { start, len, end, arena, ambient })
}

/// Close the open arena, if any, keeping what it handed out; the extent
/// that now owns it: the first granules of the run, or its last ones for
/// [`KeepEnd::High`] (the run's end is a granule boundary, so what a
/// top-down arena handed out is whole granules from there down).
fn close_keeping() -> Option<KeepExtent> {
    // SAFETY: single-threaded by contract.
    let o = unsafe { KEEP_OPEN.get() }.take()?;
    let e = extent_of(o.start, o.len, o.end, o.arena.used());
    mark(e.start as usize, e.granules as usize, true);
    note_peak();
    Some(e)
}

/// The extent an arena over the run `[start, start + len)` commits when it
/// handed out `used` bytes: the run's first whole granules, or its last ones
/// for [`KeepEnd::High`] (a top-down arena's bytes end at the run's end, a
/// granule boundary). A function of its arguments alone (see
/// [`largest_run_in`]).
fn extent_of(start: usize, len: usize, end: KeepEnd, used: usize) -> KeepExtent {
    let granules = used.div_ceil(KEEP_GRANULE);
    let start = match end {
        KeepEnd::Low => start,
        KeepEnd::High => start + len - granules,
    };
    KeepExtent { start: start as u32, granules: granules as u32 }
}

/// The longest run of free granules, in bytes: the most one compile can
/// keep right now, whatever [`keep_in_use`] says is free in total. Printed
/// beside it by the harnesses, because the two differ exactly when the
/// region is fragmented.
///
/// An open arena's handed-out granules count as held, as in
/// [`keep_in_use`]; the rest of its run is free.
pub fn keep_largest_free() -> usize {
    // SAFETY: single-threaded by contract.
    let held = unsafe { KEEP_OPEN.get() }.as_ref().map(|o| {
        let n = o.arena.used().div_ceil(KEEP_GRANULE);
        match o.end {
            KeepEnd::Low => o.start..o.start + n,
            KeepEnd::High => o.start + o.len - n..o.start + o.len,
        }
    });
    largest_run_where(|g| granule_used(g) || held.as_ref().is_some_and(|h| h.contains(&g))).1 * KEEP_GRANULE
}

fn note_peak() {
    let now = keep_in_use();
    // SAFETY: single-threaded by contract.
    let peak = unsafe { KEEP_PEAK.get() };
    if now > *peak {
        *peak = now;
    }
}

/// One compile's hold on the keep region: [`keep`] is the arena this opened
/// until it is committed or dropped.
///
/// Committing turns what the compile allocated into a [`KeepExtent`] its
/// shader owns. Dropping it uncommitted -- a compile that failed -- gives
/// everything back, which a bump arena that is never reset could not do: a
/// refused shader used to cost the region its code all the same.
pub struct KeepTxn {
    /// Test builds: the whole transaction holds [`test_lock`].
    #[cfg(test)]
    _serial: Serial,
}

impl KeepTxn {
    /// Keep what the compile allocated, as one extent.
    pub fn commit(self) -> KeepExtent {
        close_keeping().unwrap_or(KeepExtent::EMPTY)
    }
}

impl Drop for KeepTxn {
    fn drop(&mut self) {
        // SAFETY: single-threaded by contract. Nothing was marked, so
        // dropping the arena is the whole of the roll-back.
        unsafe { KEEP_OPEN.get() }.take();
    }
}

/// Open the keep arena for one compile, at the bottom of the largest free
/// run: [`begin_keep_at`]`(`[`KeepEnd::Low`]`)`, `None` for either refusal.
pub fn begin_keep() -> Option<KeepTxn> {
    begin_keep_at(KeepEnd::Low).ok()
}

/// Why [`begin_keep_at`] opened nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeepRefused {
    /// A compile already holds the keep arena.
    Open,
    /// There is no keep region: an image whose OSAL has not lent one
    /// ([`shc_online_lend_keep`]).
    NotLent,
}

/// Open the keep arena for one compile, at `end` of the largest free run
/// (see [`KeepEnd`] for which end a record belongs at).
///
/// `None` when a compile already has it open: two compiles cannot share one
/// open arena without one of them owning the other's allocations. An
/// arena [`keep`] opened outside any compile is kept first, as it always
/// was, and does not count.
pub fn begin_keep_at(end: KeepEnd) -> Result<KeepTxn, KeepRefused> {
    /* taken before the check, so the check and the open are one step */
    #[cfg(test)]
    let serial = test_lock();
    // SAFETY: single-threaded by contract.
    if let Some(o) = unsafe { KEEP_OPEN.get() } {
        if !o.ambient {
            return Err(KeepRefused::Open);
        }
        close_keeping();
    }
    if region().granules == 0 {
        return Err(KeepRefused::NotLent);
    }
    open(false, end);
    Ok(KeepTxn {
        #[cfg(test)]
        _serial: serial,
    })
}

/// Give a compiled shader's extent back to the region.
///
/// Every `ShcShader` pointing into it is dangling afterwards; the caller is
/// the reference count that knows there are none.
pub fn free_keep(ext: KeepExtent) {
    let (s, n) = (ext.start as usize, ext.granules as usize);
    debug_assert!(s + n <= region().granules);
    debug_assert!((s..s + n).all(granule_used), "an extent freed twice");
    /* Test builds: the freed bytes are poisoned (as the arena's give_back
     * poisons), so a reader that kept a pointer into a freed record reads
     * KEEP_POISON, never the old record (which the old placement left in
     * place for long) nor a plausible new one. */
    #[cfg(test)]
    // SAFETY: the extent is inside the region, and freed: nothing may read
    // it any more, which is what the poison is there to show.
    unsafe {
        core::ptr::write_bytes(region().base.add(s * KEEP_GRANULE), KEEP_POISON, n * KEEP_GRANULE)
    };
    mark(s, n, false);
}

/// The byte test builds write over a freed extent ([`free_keep`]); a word of
/// it is 0xa5a5_a5a5.
#[cfg(test)]
pub const KEEP_POISON: u8 = 0xa5;

/// Whether `p` points into the keep region: a pointer a compiled shader of
/// this back end can carry, and one no other back end's can.
pub fn in_keep(p: *const u8) -> bool {
    let r = region();
    let base = r.base as usize;
    (base..base + r.granules * KEEP_GRANULE).contains(&(p as usize))
}

/// Bytes of the keep region held right now: every extent a shader owns, and
/// what the open arena has handed out so far -- rounded up to whole granules,
/// as it will be held once the arena closes, so that closing it does not
/// change the figure.
pub fn keep_in_use() -> usize {
    let owned: usize = map().iter().map(|w| w.count_ones() as usize).sum::<usize>() * KEEP_GRANULE;
    // SAFETY: single-threaded by contract.
    owned + unsafe { KEEP_OPEN.get() }.as_ref().map_or(0, |o| o.arena.used().div_ceil(KEEP_GRANULE) * KEEP_GRANULE)
}

/// Run `f` with an [`Arena`] over the scratch region.
///
/// `None` when the region is already lent out, which means a compile was
/// started inside a compile. Nothing in this library does that, and the day
/// something does, two arenas would be handing out the same bytes and the
/// inner compile would quietly overwrite the outer one's IR — so it is refused
/// and reported instead. (A flag, not a borrow: the region is a `static`, and
/// the borrow checker has nothing to hold on to there.)
///
/// The arena is fresh on every call, which is what "reset per compile" means
/// here: a bump allocator over a buffer that starts at offset zero.
pub fn with_scratch<R>(f: impl FnOnce(&Arena<'_>) -> R) -> Option<R> {
    /* Serialised in test builds so that no test has to remember to do it; see
     * [`test_lock`]. Re-entrant, so a test that holds it around a sequence of
     * compiles does not deadlock here. Nothing of this exists in a real
     * build, where the single-threaded contract holds. */
    #[cfg(test)]
    let _serial = test_lock();

    // SAFETY: single-threaded by contract; the flag is read and written only
    // here.
    let busy = unsafe { SCRATCH_BUSY.get() };
    if *busy {
        return None;
    }
    *busy = true;
    /* Cleared by dropping, so that a panic inside `f` releases the region
     * instead of marking it busy for the rest of the process.
     *
     * That is not hypothetical tidiness: it is how one failing test became a
     * dozen. A panic skipped the clear, every later compile was refused as
     * "a compile was started inside a compile", and the run named twelve
     * failures of which eleven were the flag. The first failure is the only
     * one worth reading, and it can only be the only one if this unwinds. */
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            // SAFETY: as in the caller; single-threaded by contract.
            *unsafe { SCRATCH_BUSY.get() } = false;
        }
    }
    let _release = Release;

    /* The lent region if there is one, else this build's own. With no table
     * there is no built-in region, so a compile before the lend is refused --
     * which the caller sees as an ordinary "the compiler has no scratch"
     * failure naming the shader, not a crash. */
    let (ptr, len) = match *unsafe { LENT.get() } {
        Some(lent) => lent,
        #[cfg(any(not(feature = "no-shader-table"), feature = "host-test"))]
        None => (SCRATCH_MEM.0.get() as *mut u8, SCRATCH_BYTES),
        #[cfg(all(feature = "no-shader-table", not(feature = "host-test")))]
        None => return None,
    };
    // SAFETY: the flag above makes this the only live borrow of the region.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
    let arena = Arena::new(buf);
    let out = f(&arena);
    /* The high-water mark, not where the pointer ended: the front end's
     * scratch is given back before selection (`lower::relocate`), so the
     * two differ. */
    let used = arena.peak();
    // SAFETY: as above.
    let peak = unsafe { SCRATCH_PEAK.get() };
    if used > *peak {
        *peak = used;
    }
    // SAFETY: as above.
    *unsafe { LAST_SCRATCH_PEAK.get() } = used;
    /* `_release` clears the flag here. The arena and everything it handed out
     * are dead: `f` has returned, and its result cannot borrow the arena
     * because `R` outlives this call. */
    Some(out)
}

/// Throw away everything the keep arena holds, for a harness that measures
/// one shader at a time.
///
/// **Every `ShcShader` compiled before this call is dangling afterwards** --
/// its `usc`, uniform rows and constant file all point into the region this
/// resets. It exists because the oracle compiles a whole 324-source corpus in
/// one process, which no application does, and a corpus-total high-water mark
/// is the wrong number to size a product's arena from: what matters is the
/// largest single shader and how many an application keeps alive at once.
///
/// It is not the product's free path: that is [`free_keep`], one shader at a
/// time, when the GL core lets go of the last copy of it.
///
/// The compile cache must be cleared with it, or a later hit would hand back
/// a shader pointing into the region: [`super::backend::forget_compiled`]
/// does both, and is the one callers should use.
#[cfg(feature = "host-test")]
pub fn reset_keep() {
    note_peak();
    // SAFETY: single-threaded by contract, as everywhere in this file.
    unsafe { KEEP_OPEN.get() }.take();
    map().fill(0);
}

/// The arena a compile keeps its output in: the one [`begin_keep`] opened.
///
/// Outside a compile -- which only a test does -- this opens one of its own,
/// and what it hands out is kept for the life of the process, as everything
/// was before shaders could be freed.
///
/// Test builds refuse that unless the test asked for it
/// ([`allow_ambient_keep`]): such an arena stays open after the test, over
/// the region every later test reads, and the next compile closes it and
/// marks its granules under that test's feet. A test that compiles holds
/// [`begin_keep`] for as long as it reads the result (dropping it keeps
/// nothing). This is what made `a_high_extent_is_placed_at_the_top_of_the_
/// largest_run` flake 1 run in 3 under load (9d3dc8d2): the codegen tests
/// that compiled into `keep()` left one open.
///
/// The reference is good until the arena is closed (the compile's
/// [`KeepTxn`] committed or dropped, or the next compile opening its own);
/// what the arena hands out is good until its shader is freed.
pub fn keep() -> &'static Arena<'static> {
    // SAFETY: single-threaded by contract.
    match unsafe { KEEP_OPEN.get() } {
        Some(o) => &o.arena,
        None => {
            #[cfg(test)]
            assert!(
                AMBIENT_KEEP.with(|a| a.get()),
                "store::keep() outside a compile opens an arena that outlives this test: hold \
                 store::begin_keep() for as long as the test reads what it compiled, or ask for it with \
                 store::allow_ambient_keep()"
            );
            &open(true, KeepEnd::Low).arena
        }
    }
}

#[cfg(test)]
std::thread_local! {
    /// Whether this thread's test asked for an arena outside a compile.
    static AMBIENT_KEEP: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Tests only: let [`keep`] open an arena outside a compile while the guard
/// lives, for a test about that arena itself.
#[cfg(test)]
pub fn allow_ambient_keep() -> AmbientKeep {
    AmbientKeep(AMBIENT_KEEP.with(|a| a.replace(true)))
}

/// Tests only: the test lock and a keep transaction, for a test that
/// compiles or assembles straight into [`keep`] (not through the back end's
/// entry points, which open their own). Hold it for as long as the test
/// reads what it compiled; dropping it keeps nothing, so no arena outlives
/// the test.
#[cfg(test)]
pub fn test_keep() -> TestKeep {
    let serial = test_lock();
    let txn = begin_keep().expect("test_keep: a compile already holds the keep arena");
    TestKeep { _txn: txn, _serial: serial }
}

/// See [`test_keep`]: the transaction drops before the lock.
#[cfg(test)]
pub struct TestKeep {
    _txn: KeepTxn,
    _serial: Serial,
}

/// See [`allow_ambient_keep`]; the previous setting comes back on drop.
#[cfg(test)]
pub struct AmbientKeep(bool);

#[cfg(test)]
impl Drop for AmbientKeep {
    fn drop(&mut self) {
        AMBIENT_KEEP.with(|a| a.set(self.0));
    }
}

/// Lend the compiler the region it works in.
///
/// The caller owns the memory and must keep it alive and untouched for as long
/// as the compiler may run — on Zephyr that is a static in the OSAL, sized by
/// `CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES`.
///
/// Lending is how the size becomes a policy the product sets rather than a
/// constant the compiler carries. A shader too large for the region is refused
/// by name, which is a failure someone can read and act on; a region chosen by
/// the library would instead be either wasteful or a boot that does not
/// happen.
///
/// Returns 0, or -EINVAL for a null pointer or a region too small to be worth
/// starting: a compile that cannot get past the token buffer would report
/// "out of scratch" for every shader, which says nothing about any of them.
///
/// # Safety
/// `buf` must point at `len` writable bytes that outlive every later compile,
/// and nothing else may use them meanwhile. Single-threaded by contract, like
/// the rest of this module.
#[no_mangle]
pub unsafe extern "C" fn shc_online_lend_scratch(buf: *mut u8, len: usize) -> i32 {
    if buf.is_null() || len < MIN_SCRATCH_BYTES {
        return -crate::util::EINVAL;
    }
    // SAFETY: single-threaded by contract; written only here.
    *unsafe { LENT.get() } = Some((buf, len));
    0
}

/// Lend the compiler the region compiled shaders are kept in: what
/// `CONFIG_YAKOGL_SHADERC_KEEP_BYTES` sizes on Zephyr.
///
/// The region must hold every shader and program an application keeps alive
/// at once, in one free run per compile (see [`KEEP_BYTES`] and
/// [`KeepEnd`]); femtovg alone keeps 388,544 bytes on the V3D. How much an
/// application keeps is the product's to know, so the size is the product's
/// to set, as the scratch's is.
///
/// The region's map (one bit per 64-byte granule) is carved from its head:
/// 8 bytes per 4 KiB. Returns 0; -EINVAL for a null pointer, a pointer not
/// 8-byte aligned, or fewer than [`MIN_KEEP_BYTES`]; -EBUSY when the region
/// in use holds anything, or a compile has it open, because what it holds
/// points into it and would be lost.
///
/// # Safety
/// `buf` must point at `len` writable bytes that outlive every compiled
/// shader, and nothing else may use them. Single-threaded by contract.
#[no_mangle]
pub unsafe extern "C" fn shc_online_lend_keep(buf: *mut u8, len: usize) -> i32 {
    if buf.is_null() || !(buf as usize).is_multiple_of(8) || len < MIN_KEEP_BYTES {
        return -crate::util::EINVAL;
    }
    // SAFETY: single-threaded by contract.
    if unsafe { KEEP_OPEN.get() }.is_some() || keep_in_use() != 0 {
        return -crate::util::EBUSY;
    }
    // SAFETY: the caller's contract, checked for null and alignment above.
    let r = unsafe { carve_keep(buf, len) };
    // SAFETY: single-threaded by contract; written only here.
    *unsafe { KEEP_LENT.get() } = Some(r);
    0
}

/// A lent buffer as a keep region: the map at its head (each word covers 64
/// granules, 4 KiB, and costs 8 bytes), cleared, and the granules from the
/// first granule boundary after it.
///
/// # Safety
/// `buf` is 8-byte aligned and valid for `len` writable bytes.
unsafe fn carve_keep(buf: *mut u8, len: usize) -> KeepRegion {
    let data = |w: usize| (w * 8).div_ceil(KEEP_GRANULE) * KEEP_GRANULE;
    let mut words = len / (64 * KEEP_GRANULE + 8);
    while words > 0 && data(words) + words * 64 * KEEP_GRANULE > len {
        words -= 1;
    }
    let map = buf as *mut u64;
    // SAFETY: `[buf, buf + words * 8)` is inside the region and aligned.
    unsafe { core::ptr::write_bytes(map, 0, words) };
    // SAFETY: `data(words) + words * 4096 <= len`, so the base and every
    // granule after it are inside the region.
    KeepRegion { base: unsafe { buf.add(data(words)) }, granules: words * 64, map }
}

/// Below this a lent keep region is refused: a few shaders would fill it and
/// every later compile would read as the shader's fault. 64 KiB, mirrored as
/// the lower bound of `CONFIG_YAKOGL_SHADERC_KEEP_BYTES`'s range.
pub const MIN_KEEP_BYTES: usize = 64 * 1024;

/// The keep region, for a harness on the target: bytes held now, and the most
/// ever held. Either pointer may be null. Returns 0.
///
/// gl_test case 90 ("churn") reads it before, during and after a thousand
/// create/compile/link/draw/delete cycles: with the free path the figure held
/// now returns to where it started and the high-water stops moving; without
/// it, compiles were refused after about 200 programs.
///
/// # Safety
/// Each pointer is null or valid for one `size_t` write.
#[no_mangle]
pub unsafe extern "C" fn shc_online_keep_stats(in_use: *mut usize, peak: *mut usize) -> i32 {
    let (now, high) = (keep_in_use(), keep_used());
    // SAFETY: the caller's contract: null or valid for one write.
    unsafe {
        if !in_use.is_null() {
            in_use.write(now);
        }
        if !peak.is_null() {
            peak.write(high);
        }
    }
    0
}

/// How many compiles have run: every compile the cache could not answer,
/// whatever its outcome ([`note_compile`]).
static COMPILES: Global<u32> = Global::new(0);

/// A compile the cache could not answer is starting.
pub fn note_compile() {
    // SAFETY: single-threaded by contract.
    let n = unsafe { COMPILES.get() };
    *n = n.wrapping_add(1);
}

/// `uint32_t shc_online_compiles(void)`: how many compiles have run so far
/// (cache hits are not compiles), wrapping. A board run reads it around a
/// frame to tell a frame that compiled from one that did not.
#[no_mangle]
pub extern "C" fn shc_online_compiles() -> u32 {
    // SAFETY: single-threaded by contract.
    *unsafe { COMPILES.get() }
}

/// Below this a region is refused outright: every shader would fail the same
/// way and none of the failures would mean anything. 64 KiB is well under the
/// smallest source the corpus compiles and well over the token buffer.
///
/// `zephyr/Kconfig` mirrors this as the lower bound of
/// `YAKOGL_SHADERC_SCRATCH_BYTES`'s range, to move the refusal from boot to
/// configure time. This constant stays the authority: if the two drift, the
/// lend fails and the OSAL says so with both numbers, rather than a build
/// quietly producing an image whose every shader fails.
pub const MIN_SCRATCH_BYTES: usize = 64 * 1024;

/// The most scratch any compile has used, for sizing [`SCRATCH_BYTES`] at M4.
pub fn scratch_peak() -> usize {
    // SAFETY: single-threaded by contract.
    *unsafe { SCRATCH_PEAK.get() }
}

/// The scratch high-water of the most recent compile alone, where
/// [`scratch_peak`] is the most of any: what one shader needs, for a test
/// that pins a single large shader's footprint among other compiles.
pub fn last_scratch_peak() -> usize {
    // SAFETY: single-threaded by contract.
    *unsafe { LAST_SCRATCH_PEAK.get() }
}

/// The most the keep arena has held at once.
///
/// In a real build nothing resets it, so this is simply what it holds and
/// only ever grows. A harness that resets between shaders gets the largest
/// single shader instead -- which is what a product's arena must fit, since
/// the corpus-total is a number no application produces.
pub fn keep_used() -> usize {
    // SAFETY: single-threaded by contract.
    let peak = *unsafe { KEEP_PEAK.get() };
    peak.max(keep_in_use())
}

#[cfg(test)]
std::thread_local! {
    /// How many times THIS thread holds [`test_lock`]'s guard. One static, so
    /// that taking and dropping it count against the same number.
    static SERIAL_DEPTH: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Serialises the tests that touch these statics.
///
/// The library is single-threaded by contract, and [`with_scratch`] enforces
/// that contract rather than implementing a lock: a second caller is *refused*
/// so that two arenas can never hand out the same bytes. The test harness runs
/// tests on several threads, which breaks the contract, so they refuse each
/// other at random -- which is `with_scratch` doing its job, and reads as a
/// flake.
///
/// **This used to be a guard each test had to remember to take**, and one did
/// not: the suite failed about one run in three with `-EINVAL` and "a compile
/// was started inside a compile", which is a compiler error message for a
/// harness problem. Finding *which* test forgot is beside the point -- a rule
/// a person has to remember is one the next test will forget again. So
/// [`with_scratch`] takes this itself, and the guard is **re-entrant per
/// thread**: a test that needs a whole SEQUENCE of compiles to be atomic (one
/// stage then the other, then a check on the first) still takes it explicitly,
/// and the inner acquisition inside each compile sees the thread already
/// holds it and does not deadlock.
///
/// The reentrancy *refusal* is untouched: it is the `SCRATCH_BUSY` flag, not
/// this lock, so a genuine compile-inside-a-compile is still refused and
/// [`tests::a_compile_inside_a_compile_is_refused_rather_than_aliased`] still
/// proves it.
/// `pub(crate)` so that a test elsewhere in the crate that compiles through
/// the in-process compiler can take it too: the V3D backend's rig builds and
/// releases its clear program (src/v3d/backend_tests.rs).
#[cfg(test)]
pub(crate) fn test_lock() -> Serial {
    let outermost = SERIAL_DEPTH.with(|d| {
        let n = d.get();
        d.set(n + 1);
        n == 0
    });
    /* Only the outermost acquisition on a thread takes the mutex; the inner
     * ones ride on it. Poison is not interesting: a test that panicked while
     * holding it has already failed the run. */
    Serial(outermost.then(|| SERIAL.lock().unwrap_or_else(|e| e.into_inner())))
}

/// The guard [`test_lock`] returns. Dropping it releases this thread's
/// outermost acquisition and, with it, the mutex.
///
/// The `MutexGuard` is never read: it is held so that dropping it unlocks,
/// which is the whole of its job. `None` on an inner acquisition, where some
/// outer guard on this thread holds the mutex already.
#[cfg(test)]
pub(crate) struct Serial(#[allow(dead_code)] Option<std::sync::MutexGuard<'static, ()>>);

#[cfg(test)]
impl Drop for Serial {
    fn drop(&mut self) {
        SERIAL_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compile_starts_with_the_whole_scratch_region() {
        let _serial = test_lock();
        for _ in 0..2 {
            let used = with_scratch(|a| {
                assert_eq!(a.used(), 0, "every compile starts at offset zero");
                assert_eq!(a.left(), SCRATCH_BYTES);
                let _ = a.alloc_filled(1024, 0u8).unwrap();
                a.used()
            })
            .unwrap();
            assert!(used >= 1024);
        }
    }

    #[test]
    fn the_scratch_peak_survives_the_compiles_that_set_it() {
        let _serial = test_lock();
        /* M4 sizes SCRATCH_BYTES from this, so it has to outlive the arena
         * that measured it: a peak read after the buffer went away is the only
         * number there is. */
        with_scratch(|a| {
            let _ = a.alloc_filled(4096, 0u8).unwrap();
        })
        .unwrap();
        let after_big = scratch_peak();
        assert!(after_big >= 4096, "{after_big}");
        with_scratch(|a| {
            let _ = a.alloc_filled(8, 0u8).unwrap();
        })
        .unwrap();
        assert_eq!(scratch_peak(), after_big, "a smaller compile does not lower the peak");
    }

    #[test]
    fn a_compile_inside_a_compile_is_refused_rather_than_aliased() {
        let _serial = test_lock();
        /* Two arenas over one static region would hand out the same bytes, and
         * the inner compile would quietly overwrite the outer one's IR. The
         * inner call gets None, which its caller reports. */
        let inner = with_scratch(|_outer| with_scratch(|_| 1u32)).unwrap();
        assert!(inner.is_none(), "the region cannot be lent out twice at once");
        assert!(with_scratch(|_| 1u32).is_some(), "and the refusal did not leave it locked");
    }

    #[test]
    fn keep_outside_a_compile_is_the_same_arena_every_time_and_never_rewinds() {
        let _serial = test_lock();
        let _ambient = allow_ambient_keep(); /* this test is about that arena */
        let before = keep_in_use();
        let s = keep().alloc_copy(b"v_uv").unwrap();
        assert_eq!(s, b"v_uv");
        /* A granule more, so the figure -- in whole granules -- must move. */
        let _ = keep().alloc_filled(KEEP_GRANULE, 0u8).unwrap();
        let after = keep_in_use();
        assert!(after > before, "{before} -> {after}");
        assert!(core::ptr::eq(keep(), keep()), "a second keep() is the same arena, not a fresh one");
        /* And a compile opening its own arena keeps what that one handed
         * out rather than handing the bytes to the compile. */
        let ptr = s.as_ptr();
        let txn = begin_keep().expect("no compile is open");
        let _ = keep().alloc_filled(64, 0xffu8).unwrap();
        drop(txn);
        // SAFETY: the ambient arena's bytes are kept for the process.
        assert_eq!(unsafe { core::slice::from_raw_parts(ptr, 4) }, b"v_uv");
        assert!(keep_in_use() >= after);
    }

    /// The free path: an extent a compile committed is the region's again
    /// once freed, and the next compile gets the same bytes. A roll-back
    /// (a compile that failed) takes nothing at all.
    ///
    /// Reverting `free_keep` to a no-op fails the second assertion.
    #[test]
    fn a_freed_extent_is_reused_and_a_rolled_back_compile_takes_nothing() {
        let _serial = test_lock();
        let base = {
            let t = begin_keep().unwrap();
            let e = t.commit();
            free_keep(e);
            keep_in_use()
        };
        let t = begin_keep().unwrap();
        let first = keep().alloc_filled(1000, 1u8).unwrap().as_ptr();
        let e = t.commit();
        assert_eq!(e.bytes(), 1024, "1000 bytes is 16 granules");
        assert_eq!(keep_in_use(), base + 1024);
        free_keep(e);
        assert_eq!(keep_in_use(), base, "the extent is given back");
        let t = begin_keep().unwrap();
        let again = keep().alloc_filled(1000, 2u8).unwrap().as_ptr();
        drop(t);
        assert_eq!(keep_in_use(), base, "a dropped transaction keeps nothing");
        /* The largest free run is where it was, so the same bytes. */
        assert_eq!(first, again);
        let t = begin_keep().unwrap();
        assert!(begin_keep().is_none(), "one compile at a time holds the arena");
        drop(t);
    }

    /// A [`KeepEnd::High`] extent is the LAST granules of the largest free
    /// run and a [`KeepEnd::Low`] one the first; freed, the high one rejoins
    /// the run. Revert check: `extent_of`'s `High` arm returning `start`
    /// fails the first assertion.
    ///
    /// Checked on a map and a buffer of the test's own, through the same two
    /// functions the store places with (`largest_run_in`, `extent_of`) and
    /// the arena it opens (`Arena::new_top_down`), and nothing global: under
    /// load it read the shared region, which other test threads free into
    /// and hold open (a GL-level compile opens its keep transaction before it
    /// takes the test lock), and failed in other teams' gates three times on
    /// 2026-09-24, after 9d3dc8d2 had settled the ambient arena.
    #[test]
    fn a_high_extent_is_placed_at_the_top_of_the_largest_run() {
        const GRANULES: usize = 128;
        let mut used = [false; GRANULES];
        used[..10].fill(true);
        used[100..110].fill(true);
        let run = largest_run_in(GRANULES, |g| used[g]);
        assert_eq!(run, (10, 90), "the free run between the two held blocks");
        let (start, len) = run;
        /* 1000 bytes from the top of the run, through the arena the store
         * opens there */
        let mut buf = std::vec![0u8; len * KEEP_GRANULE];
        let base = buf.as_ptr() as usize;
        let hi = {
            let a = Arena::new_top_down(&mut buf);
            let p = a.alloc_filled(1000, 1u8).unwrap().as_ptr() as usize;
            (p, a.used())
        };
        let e_hi = extent_of(start, len, KeepEnd::High, hi.1);
        assert_eq!((e_hi.start as usize, e_hi.granules as usize), (start + len - 16, 16), "the run's last 16 granules");
        let rel = (e_hi.start as usize - start) * KEEP_GRANULE;
        assert!(hi.0 >= base + rel && hi.0 + 1000 <= base + len * KEEP_GRANULE, "the bytes are inside the extent");
        let e_lo = extent_of(start, len, KeepEnd::Low, 100);
        assert_eq!((e_lo.start as usize, e_lo.granules as usize), (start, 2), "the low one at the run's start");
        let hold = |used: &mut [bool; GRANULES], e: KeepExtent, on: bool| {
            used[e.start as usize..(e.start + e.granules) as usize].fill(on)
        };
        hold(&mut used, e_hi, true);
        hold(&mut used, e_lo, true);
        assert_eq!(largest_run_in(GRANULES, |g| used[g]), (start + 2, len - 2 - 16));
        hold(&mut used, e_hi, false);
        assert_eq!(largest_run_in(GRANULES, |g| used[g]), (start + 2, len - 2), "the high one rejoined the run");
        hold(&mut used, e_lo, false);
        assert_eq!(largest_run_in(GRANULES, |g| used[g]), run);
    }

    /// femtovg's 30 programs held at once, replayed with the sizes measured
    /// on the V3D (v3d-slint, 2026-09-24, ea8ab0d6): the vertex stage record
    /// 1,984 bytes (compiled once, held by every program), each fragment
    /// stage record 10,624 (freed when femtovg deletes the shader after the
    /// link), each program record 13,952. Each program: compile the
    /// fragment stage, link, delete the shader.
    ///
    /// With a record's lifetime choosing its end, all 30 fit and the region
    /// holds them packed: the largest free run shrinks by exactly what is
    /// held. With every record at the bottom (the placement before
    /// [`KeepEnd`], [`set_single_ended`]) the same sequence runs out while
    /// most of the region is free -- at program 22 from an empty region,
    /// the program the board and the host both named.
    #[test]
    fn femtovgs_thirty_programs_fit_when_each_lifetime_has_its_end() {
        let _serial = test_lock();
        struct Restore(bool);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_single_ended(self.0);
            }
        }
        let _restore = Restore(set_single_ended(false));
        const VS: usize = 1_984;
        const FS: usize = 10_624;
        const PROGRAM: usize = 13_952;
        let place = |end: KeepEnd, bytes: usize| -> Option<KeepExtent> {
            let t = begin_keep_at(end).ok()?;
            keep().alloc_filled(bytes, 0u8).ok()?;
            Some(t.commit())
        };
        /* One run of femtovg's sequence: how many programs it held, and
         * the region's figures at the end, with everything given back. */
        let replay = |single: bool| -> (usize, usize, usize, usize) {
            set_single_ended(single);
            let mut held = std::vec::Vec::new();
            let vs = place(KeepEnd::High, VS).expect("the vertex stage fits an empty region");
            let mut n = 0;
            for _ in 0..30 {
                let Some(fs) = place(KeepEnd::High, FS) else { break };
                let prog = place(KeepEnd::Low, PROGRAM);
                free_keep(fs);
                let Some(prog) = prog else { break };
                held.push(prog);
                n += 1;
            }
            let (in_use, largest) = (keep_in_use(), keep_largest_free());
            for e in held {
                free_keep(e);
            }
            free_keep(vs);
            (n, in_use, largest, keep_in_use())
        };
        let (before, largest_before) = (keep_in_use(), keep_largest_free());
        assert!(
            largest_before >= VS + 30 * PROGRAM + FS,
            "the region must start with room for the replay: largest free run {largest_before}, {before} in use"
        );
        let (n, in_use, largest, after) = replay(false);
        println!("two ends: {n} programs, {in_use} in use, largest free run {largest} (before: {largest_before})");
        assert_eq!(n, 30, "every program fits");
        assert_eq!(in_use, before + VS + 30 * PROGRAM, "what is held is the programs and the vertex stage");
        assert_eq!(largest, largest_before - VS - 30 * PROGRAM, "and it is held packed: no hole was left");
        assert_eq!(after, before, "everything was given back");
        let (n, in_use, largest, after) = replay(true);
        println!("one end: {n} programs, {in_use} in use, largest free run {largest}");
        assert!(n < 30, "the old placement is the negative control and must run out");
        if before == 0 {
            assert_eq!(n, 21, "from an empty region, program 22 is the first that does not fit");
        }
        assert_eq!(after, before);
    }

    /// A test that calls `keep()` outside a compile, without asking, is
    /// stopped at the call: the arena it would open outlives the test (see
    /// `keep`). Every lib test that did so was moved onto `test_keep()` or a
    /// `begin_keep()` it holds, and the suites pass three runs in three.
    #[test]
    #[should_panic(expected = "store::keep() outside a compile opens an arena that outlives this test")]
    fn keep_outside_a_compile_is_refused_in_a_test_that_did_not_ask() {
        let _serial = test_lock();
        close_keeping(); /* an arena a test that asked may have left */
        let _ = keep();
    }

    #[test]
    fn what_keep_hands_out_survives_a_later_compile() {
        let _serial = test_lock();
        let _ambient = allow_ambient_keep(); /* this test is about that arena */
        /* The property the whole design rests on: a ShcShader points into
         * `keep`, and the GL core reads it long after the compile returned. */
        let kept = keep().alloc_copy(&[0xdead_beefu32, 0x1234_5678]).unwrap();
        let ptr = kept.as_ptr();
        with_scratch(|a| {
            let _ = a.alloc_filled(SCRATCH_BYTES / 2, 0xffu8).unwrap();
        })
        .unwrap();
        // SAFETY: `kept` is in the keep arena, which is never reset, so the
        // pointer is still good and nothing else has been given those bytes.
        assert_eq!(unsafe { core::slice::from_raw_parts(ptr, 2) }, [0xdead_beefu32, 0x1234_5678]);
    }

    #[test]
    fn an_over_large_request_is_refused_and_takes_nothing() {
        let _serial = test_lock();
        with_scratch(|a| {
            let before = a.used();
            assert!(a.alloc_filled(SCRATCH_BYTES + 1, 0u8).is_err(), "more than the region holds");
            assert_eq!(a.used(), before, "a refused allocation consumes nothing");
        })
        .unwrap();
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    /// What gl_test case 90 reads on the target is what the Rust side
    /// measures, and a null pointer is simply not written.
    #[test]
    fn the_c_entry_reports_the_region_as_rust_sees_it() {
        let _serial = test_lock();
        let (mut now, mut high) = (usize::MAX, usize::MAX);
        // SAFETY: two valid out pointers.
        assert_eq!(unsafe { shc_online_keep_stats(&mut now, &mut high) }, 0);
        assert_eq!((now, high), (keep_in_use(), keep_used()));
        assert!(high >= now);
        // SAFETY: null pointers are allowed.
        assert_eq!(unsafe { shc_online_keep_stats(core::ptr::null_mut(), core::ptr::null_mut()) }, 0);
    }
}

#[cfg(test)]
mod lend_tests {
    use super::*;

    /// A lent region is used in preference to the built-in one, so the path an
    /// image takes is exercised by a host build rather than only on a board.
    #[test]
    fn a_lent_region_is_the_one_the_compiler_works_in() {
        let _serial = test_lock();
        let mut mine = std::vec![0u8; MIN_SCRATCH_BYTES * 2];
        let (ptr, len) = (mine.as_mut_ptr(), mine.len());
        // SAFETY: `mine` outlives the call below and nothing else touches it.
        assert_eq!(unsafe { shc_online_lend_scratch(ptr, len) }, 0);

        let seen = with_scratch(|a| a.used() + a.left()).expect("a lent region compiles");
        assert_eq!(seen, len, "the compiler did not work in the lent region");

        // SAFETY: single-threaded under the test lock.
        *unsafe { LENT.get() } = None;
    }

    /// A lent keep region is where compiles keep their output: its map is at
    /// its head, its granules after it, both inside the buffer, and a record
    /// placed at either end lands inside them. Swapped in directly (the C
    /// entry refuses while the built-in region holds anything, and other
    /// tests in this binary leave bytes kept there for good), and put back.
    /// Puts the keep region back as it was, including through a panic: a
    /// test that left a freed buffer lent would fail every later test for a
    /// reason nothing in them explains.
    struct KeepLendBack(Option<KeepRegion>);
    impl Drop for KeepLendBack {
        fn drop(&mut self) {
            // SAFETY: single-threaded under the test lock.
            *unsafe { KEEP_LENT.get() } = self.0;
        }
    }

    #[test]
    fn a_lent_keep_region_is_the_one_shaders_are_kept_in() {
        let _serial = test_lock();
        let mut mine = std::vec![0u64; (MIN_KEEP_BYTES * 2) / 8];
        let (buf, len) = (mine.as_mut_ptr() as *mut u8, mine.len() * 8);
        // SAFETY: an aligned buffer this test owns for its whole length.
        let r = unsafe { carve_keep(buf, len) };
        let base = buf as usize;
        assert_eq!(r.granules % 64, 0, "whole map words");
        assert!(r.map as usize == base && (r.base as usize) >= base + r.granules / 8, "the map, then the granules");
        assert!(r.base as usize + r.granules * KEEP_GRANULE <= base + len, "inside the buffer");
        assert_eq!(r.granules, 1984, "128 KiB less its 248-byte map, rounded down to whole map words");
        /* An arena another test opened outside a compile (`keep()`) is over
         * the built-in region: close it, keeping its bytes there, before the
         * region changes under it. The C entry refuses (-EBUSY) instead. */
        close_keeping();
        // SAFETY: single-threaded under the test lock; put back by `_back`.
        let _back = KeepLendBack(unsafe { KEEP_LENT.get() }.replace(r));
        assert_eq!(keep_largest_free(), r.granules * KEEP_GRANULE, "an empty region is one run");
        let t = begin_keep_at(KeepEnd::High).unwrap();
        let p = keep().alloc_filled(100, 3u8).unwrap().as_ptr();
        let hi = t.commit();
        let t = begin_keep_at(KeepEnd::Low).unwrap();
        let q = keep().alloc_filled(100, 4u8).unwrap().as_ptr();
        let lo = t.commit();
        assert!(in_keep(p) && in_keep(q), "both records are in the lent region");
        assert_eq!((lo.start, hi.start as usize + hi.granules as usize), (0, r.granules), "at its two ends");
        free_keep(hi);
        free_keep(lo);
        assert_eq!(keep_in_use(), 0);
        drop(_back);
        drop(mine);
    }

    /// shc_online_compiles counts a compile the cache could not answer, and
    /// not one it answered.
    #[test]
    fn the_compile_count_counts_compiles_not_cache_hits() {
        let _serial = test_lock();
        let f = super::super::backend::SHC_ONLINE.compile.unwrap();
        /* Text no other test compiles, so the first compile is a miss. */
        let src = b"precision mediump float; void main(){ gl_FragColor = vec4(0.5); } // compile count\n\0";
        let compile = || {
            let mut sh = crate::shaderc::ShcShader::ZERO;
            // SAFETY: a NUL-terminated source and a real output; no log.
            let rc = unsafe {
                f(
                    crate::shaderc::SHC_STAGE_FRAGMENT,
                    src.as_ptr() as *const core::ffi::c_char,
                    &mut sh,
                    core::ptr::null_mut(),
                    0,
                )
            };
            assert_eq!(rc, 0);
            sh
        };
        let n0 = shc_online_compiles();
        let a = compile();
        assert_eq!(shc_online_compiles(), n0.wrapping_add(1), "a miss is one compile");
        let b = compile();
        assert_eq!(shc_online_compiles(), n0.wrapping_add(1), "a cache hit is not a compile");
        crate::shaderc::release_shader(&b);
        crate::shaderc::release_shader(&a);
    }

    /// An image that lent no keep region (its built-in one compiled out) has
    /// zero granules: every compile is refused by name, "no keep region",
    /// -ENOMEM, and never read as "a compile inside a compile".
    #[test]
    fn with_no_keep_region_a_compile_is_refused_by_name() {
        let _serial = test_lock();
        let none = KeepRegion {
            base: core::ptr::NonNull::dangling().as_ptr(),
            granules: 0,
            map: core::ptr::NonNull::dangling().as_ptr(),
        };
        close_keeping(); /* as above */
        // SAFETY: single-threaded under the test lock; put back by `_back`.
        let _back = KeepLendBack(unsafe { KEEP_LENT.get() }.replace(none));
        assert_eq!(begin_keep_at(KeepEnd::High).err(), Some(KeepRefused::NotLent));
        let mut log = [0 as core::ffi::c_char; 256];
        let mut sh = crate::shaderc::ShcShader::ZERO;
        /* Text no other test compiles, so the cache cannot answer instead. */
        let src = b"precision mediump float; void main(){ gl_FragColor = vec4(0.25); } // no keep region\n\0";
        let f = super::super::backend::SHC_ONLINE.compile.unwrap();
        // SAFETY: a NUL-terminated source, a real output and log.
        let rc = unsafe {
            f(
                crate::shaderc::SHC_STAGE_FRAGMENT,
                src.as_ptr() as *const core::ffi::c_char,
                &mut sh,
                log.as_mut_ptr(),
                log.len(),
            )
        };
        // SAFETY: the writer NUL-terminated the log.
        let text = std::string::String::from_utf8_lossy(unsafe { crate::util::cstr_bytes(log.as_ptr()) }).into_owned();
        assert_eq!(rc, -crate::util::ENOMEM, "{text}");
        assert!(text.contains("no keep region"), "{text}");
    }

    /// The C entry: refused for a null, misaligned or too small region, and
    /// while the region in use holds a record; accepted otherwise.
    #[test]
    fn the_keep_lend_refuses_what_it_cannot_use() {
        let _serial = test_lock();
        let mut mine = std::vec![0u64; MIN_KEEP_BYTES / 8];
        let (buf, len) = (mine.as_mut_ptr() as *mut u8, mine.len() * 8);
        // SAFETY: single-threaded under the test lock.
        let back = KeepLendBack(*unsafe { KEEP_LENT.get() });
        // SAFETY: the pointers below are refused before any use.
        unsafe {
            assert_eq!(shc_online_lend_keep(core::ptr::null_mut(), len), -crate::util::EINVAL);
            assert_eq!(shc_online_lend_keep(buf.add(1), len - 8), -crate::util::EINVAL, "misaligned");
            assert_eq!(shc_online_lend_keep(buf, len - 8), -crate::util::EINVAL, "below MIN_KEEP_BYTES");
        }
        let t = begin_keep().unwrap();
        let _ = keep().alloc_filled(10, 0u8).unwrap();
        let e = t.commit();
        // SAFETY: refused: a record is held.
        assert_eq!(unsafe { shc_online_lend_keep(buf, len) }, -crate::util::EBUSY, "a held record points into it");
        free_keep(e);
        if keep_in_use() == 0 {
            // SAFETY: `mine` outlives the lend, which is taken back below.
            assert_eq!(unsafe { shc_online_lend_keep(buf, len) }, 0);
            assert!(in_keep(buf.wrapping_add(len / 2)), "the lent region is in use");
        } else {
            std::println!("the accepting lend is not checked: earlier tests keep {} bytes for good", keep_in_use());
        }
        drop(back);
        drop(mine);
    }

    /// Too small is refused when it is lent, not when the first shader
    /// arrives: every shader would fail the same way and none of those
    /// failures would say anything about the shader.
    #[test]
    fn a_useless_region_is_refused_at_the_lend() {
        let mut tiny = std::vec![0u8; 8];
        // SAFETY: as above.
        let rc = unsafe { shc_online_lend_scratch(tiny.as_mut_ptr(), tiny.len()) };
        assert_eq!(rc, -crate::util::EINVAL);
        // SAFETY: as above.
        assert_eq!(unsafe { shc_online_lend_scratch(core::ptr::null_mut(), 1 << 20) }, -crate::util::EINVAL);
    }
}
