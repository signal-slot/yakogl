// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Small shared helpers: the global-state cell, C string handling, a fixed
//! formatting buffer, the C heap, and errno values.

use core::cell::UnsafeCell;
use core::ffi::{c_char, c_void};
use core::fmt;
use core::mem::MaybeUninit;

/* ---- errno values (negative returns follow the C library's convention) ---- */
/// errno numbers, one table per C library: the driver crate's file, shared
/// so both crates read one table (src/osal/OSAL_NOTES.md, "errno numbers at
/// the C boundaries").
#[path = "../driver/rust/src/errno_numbers.rs"]
pub mod errno_numbers;

/// The numbering of the C library on the other side of the OSAL: the Zephyr
/// OSALs return Zephyr's numbers (`ENOTSUP` 134, `ETIMEDOUT` 116), the others
/// Linux's (`linux.c`; `baremetal.c`'s literal -95).
pub const ERRNO: errno_numbers::Numbering = errno_numbers::numbering(cfg!(feature = "osal-zephyr"));
pub const ENOENT: i32 = ERRNO.enoent;
pub const EIO: i32 = ERRNO.eio;
pub const ENOMEM: i32 = ERRNO.enomem;
pub const EBUSY: i32 = ERRNO.ebusy;
pub const ENODEV: i32 = ERRNO.enodev;
pub const EINVAL: i32 = ERRNO.einval;
pub const ENOSPC: i32 = ERRNO.enospc;
pub const ENOTSUP: i32 = ERRNO.enotsup;
pub const ETIMEDOUT: i32 = ERRNO.etimedout;

/// Global mutable state.
///
/// YakoGL is single-threaded by contract: one thread issues every GL/EGL call
/// (the C library this replaces kept the same state in plain `static`s). The
/// cell makes that contract explicit instead of scattering `static mut`.
///
/// `zeroed()` places the value in `.bss`; it is only used with types for which
/// the all-zero bit pattern is valid (integers, floats, `bool`, raw pointers,
/// `Option<fn>`, and arrays/structs of those).
#[repr(transparent)]
pub struct Global<T>(UnsafeCell<MaybeUninit<T>>);

// SAFETY: single-threaded by contract (see above).
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(v: T) -> Self {
        Global(UnsafeCell::new(MaybeUninit::new(v)))
    }

    /// All-zero initial value (in `.bss`). The caller guarantees zero is a
    /// valid `T`.
    pub const fn zeroed() -> Self {
        Global(UnsafeCell::new(MaybeUninit::zeroed()))
    }

    pub const fn as_ptr(&self) -> *mut T {
        self.0.get() as *mut T
    }

    /// # Safety
    /// No other reference obtained from this cell may be live while the
    /// returned one is used (entry points take it once and pass it down).
    #[allow(clippy::mut_from_ref)]
    #[inline]
    pub unsafe fn get(&self) -> &mut T {
        &mut *self.as_ptr()
    }
}

/* ---- the C heap -----------------------------------------------------------
 * The few variable-sized host allocations (shader source copies, draw-time
 * staging) use the platform's malloc like the C library did; the crate does
 * not pull in `alloc` or define a global allocator, so it links cleanly into
 * Rust applications too. */
extern "C" {
    pub fn malloc(size: usize) -> *mut c_void;
    pub fn calloc(n: usize, size: usize) -> *mut c_void;
    pub fn realloc(p: *mut c_void, size: usize) -> *mut c_void;
    pub fn free(p: *mut c_void);
}

/// A growable byte buffer on the C heap (staging storage that keeps its
/// capacity between uses).
pub struct CBuf {
    ptr: *mut u8,
    cap: usize,
}

impl CBuf {
    pub const fn new() -> Self {
        CBuf { ptr: core::ptr::null_mut(), cap: 0 }
    }

    /// Make room for at least `bytes`; contents are NOT preserved across a
    /// growth. Returns null on allocation failure (capacity is then 0).
    pub fn reserve(&mut self, bytes: usize) -> *mut u8 {
        if bytes > self.cap {
            // SAFETY: ptr is null or a live malloc block owned by self.
            unsafe {
                free(self.ptr as *mut c_void);
                self.ptr = malloc(bytes) as *mut u8;
            }
            self.cap = if self.ptr.is_null() { 0 } else { bytes };
        }
        self.ptr
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn release(&mut self) {
        // SAFETY: ptr is null or a live malloc block owned by self.
        unsafe { free(self.ptr as *mut c_void) };
        self.ptr = core::ptr::null_mut();
        self.cap = 0;
    }
}

impl Default for CBuf {
    fn default() -> Self {
        Self::new()
    }
}

/* ---- application pointers ------------------------------------------------
 * The GL/EGL ABI does not promise that a GLfloat* / GLint* / pointer array an
 * application passes is aligned for its type (a byte buffer cast to float*
 * is common), and aarch64-unknown-none builds with +strict-align. A typed
 * reference or slice over such memory is undefined behaviour (the dev
 * profile aborts in slice::from_raw_parts); every typed element is read or
 * written by value instead. Pointer arithmetic itself needs no alignment. */

/// Element `i` of an application array.
///
/// # Safety
/// `p` must be valid for reading `i + 1` elements (any alignment).
#[inline]
pub unsafe fn app_get<T: Copy>(p: *const T, i: usize) -> T {
    p.add(i).read_unaligned()
}

/// Store `v` as element `i` of an application array.
///
/// # Safety
/// `p` must be valid for writing `i + 1` elements (any alignment).
#[inline]
pub unsafe fn app_set<T: Copy>(p: *mut T, i: usize, v: T) {
    p.add(i).write_unaligned(v)
}

/// Store `v` through an optional application output pointer (null: nothing).
///
/// # Safety
/// `p` must be null or valid for writing one element (any alignment).
#[inline]
pub unsafe fn app_put<T: Copy>(p: *mut T, v: T) {
    if !p.is_null() {
        p.write_unaligned(v)
    }
}

/* ---- C strings ----------------------------------------------------------- */

/// Length of a NUL-terminated C string.
///
/// # Safety
/// `s` must be non-null and NUL-terminated.
pub unsafe fn cstr_len(s: *const c_char) -> usize {
    let mut n = 0;
    while *s.add(n) != 0 {
        n += 1;
    }
    n
}

/// The bytes of a NUL-terminated C string (without the NUL); empty for null.
///
/// # Safety
/// `s` must be null or NUL-terminated and outlive `'a`.
pub unsafe fn cstr_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    core::slice::from_raw_parts(s as *const u8, cstr_len(s))
}

/// The bytes of a fixed-size, NUL-padded name field up to its first NUL.
pub fn name_bytes(field: &[c_char]) -> &[u8] {
    let n = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    // SAFETY: c_char and u8 have the same layout.
    unsafe { core::slice::from_raw_parts(field.as_ptr() as *const u8, n) }
}

/// Copy `src` into a fixed-size name field, truncating, always NUL-terminated.
pub fn set_name(field: &mut [c_char], src: &[u8]) {
    if field.is_empty() {
        return;
    }
    let n = src.len().min(field.len() - 1);
    for (d, s) in field.iter_mut().zip(src[..n].iter()) {
        *d = *s as c_char;
    }
    for d in field[n..].iter_mut() {
        *d = 0;
    }
}

/// Build a NUL-padded name field at compile time (generated tables).
pub const fn cname<const N: usize>(s: &str) -> [c_char; N] {
    let b = s.as_bytes();
    let mut out = [0 as c_char; N];
    let mut i = 0;
    while i < b.len() && i + 1 < N {
        out[i] = b[i] as c_char;
        i += 1;
    }
    out
}

/// Copy a byte string into a caller-provided C buffer the way the GL "get
/// string" calls do: at most `buf_size - 1` bytes plus a NUL; `length` (if
/// non-null) receives the bytes written without the NUL.
///
/// # Safety
/// `buf` must be null or valid for `buf_size` bytes; `length` null or valid.
pub unsafe fn copy_out_str(src: &[u8], buf_size: i32, length: *mut i32, buf: *mut c_char) {
    let mut n = 0usize;
    if !buf.is_null() && buf_size > 0 {
        n = src.len().min(buf_size as usize - 1);
        core::ptr::copy_nonoverlapping(src.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    app_put(length, n as i32);
}

/* ---- formatting into a fixed buffer -------------------------------------- */

/// `core::fmt::Write` into a stack buffer; always NUL-terminated, silently
/// truncating (log lines and info logs). `N` must be at least 1 (the NUL): a
/// `FmtBuf<0>` fails to compile where `new` is instantiated, since its C view
/// (`as_cstr_ptr`) would point at no terminator at all.
pub struct FmtBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FmtBuf<N> {
    const HAS_ROOM_FOR_THE_NUL: () = assert!(N > 0, "FmtBuf<0> has no room for its NUL terminator");

    pub const fn new() -> Self {
        #[allow(clippy::let_unit_value)]
        let () = Self::HAS_ROOM_FOR_THE_NUL;
        FmtBuf { buf: [0; N], len: 0 }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// NUL-terminated view for C consumers.
    pub fn as_cstr_ptr(&self) -> *const c_char {
        self.buf.as_ptr() as *const c_char
    }

    /// The text up to its first NUL (formatted text may itself contain one).
    pub fn as_cstr(&self) -> &core::ffi::CStr {
        // The buffer always holds a NUL at or before `len` (see write_str).
        core::ffi::CStr::from_bytes_until_nul(&self.buf).unwrap_or(c"")
    }
}

impl<const N: usize> Default for FmtBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Write for FmtBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        /* N >= 1 (new); the checked form keeps a violation from indexing */
        let Some(last) = N.checked_sub(1) else {
            return Ok(());
        };
        let room = last.saturating_sub(self.len);
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        self.buf[self.len] = 0;
        Ok(())
    }
}

/// `core::fmt::Write` into a caller-provided C buffer (`char *log, size_t
/// log_sz`): NUL-terminated, truncating, appending after what is there.
pub struct CStrWriter {
    buf: *mut c_char,
    cap: usize,
    len: usize,
}

impl CStrWriter {
    /// # Safety
    /// `buf` must be null or valid for `cap` bytes for the writer's lifetime.
    pub unsafe fn new(buf: *mut c_char, cap: usize) -> Self {
        if !buf.is_null() && cap > 0 {
            *buf = 0;
        }
        CStrWriter { buf, cap, len: 0 }
    }
}

impl fmt::Write for CStrWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.buf.is_null() || self.cap == 0 {
            return Ok(());
        }
        let room = (self.cap - 1).saturating_sub(self.len);
        let n = s.len().min(room);
        // SAFETY: len + n < cap, inside the caller's buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), self.buf.add(self.len) as *mut u8, n);
            self.len += n;
            *self.buf.add(self.len) = 0;
        }
        Ok(())
    }
}

/// Display adapter for application byte strings (GLSL identifiers, shader
/// source lines, info logs). Valid UTF-8 prints as is, like the C library's
/// `%s` (a shader comment in Japanese reaches the log unchanged); a byte that
/// is not part of valid UTF-8 cannot pass through `core::fmt` and prints as
/// `?`, as does a NUL (where `%s` would have stopped).
pub struct Bytes<'a>(pub &'a [u8]);

impl fmt::Display for Bytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        for chunk in self.0.utf8_chunks() {
            for (i, part) in chunk.valid().split('\0').enumerate() {
                if i != 0 {
                    f.write_char('?')?;
                }
                f.write_str(part)?;
            }
            for _ in chunk.invalid() {
                f.write_char('?')?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write as _;
    use std::format;

    /// What an `osal-zephyr` build's errno constants are: the numbers the
    /// Zephyr OSALs return (built against Zephyr's libc: ETIMEDOUT 116,
    /// ENOTSUP 134; OSAL_NOTES.md has the header lines). With one numbering
    /// for every OSAL (Linux's, as before this test) the build read Zephyr's
    /// ESHUTDOWN (110) as ETIMEDOUT and never saw zephyr.c's -134 "no vsync"
    /// as -ENOTSUP in eglSwapBuffers.
    #[test]
    fn zephyr_osal_reads_zephyr_numbers() {
        let z = errno_numbers::numbering(true);
        assert_eq!(z.etimedout, 116, "a Zephyr OSAL's -ETIMEDOUT is -116");
        assert_ne!(z.etimedout, 110, "110 is Zephyr's ESHUTDOWN, not a timeout");
        assert_eq!(z.eshutdown, 110);
        assert_eq!(z.enotsup, 134, "zephyr.c's default wait_vblank returns -134");
        let l = errno_numbers::numbering(false);
        assert_eq!((l.etimedout, l.enotsup), (110, 95), "linux.c (glibc), baremetal.c's -95");
        assert_eq!(ERRNO, errno_numbers::numbering(cfg!(feature = "osal-zephyr")));
        assert_eq!((ETIMEDOUT, ENOTSUP), (ERRNO.etimedout, ERRNO.enotsup));
    }

    fn field_bytes(f: &[c_char]) -> std::vec::Vec<u8> {
        f.iter().map(|&c| c as u8).collect()
    }

    #[test]
    fn cname_pads_a_short_name_with_nuls() {
        let f: [c_char; 8] = cname("a_pos");
        assert_eq!(field_bytes(&f), b"a_pos\0\0\0");
    }

    #[test]
    fn cname_truncates_to_leave_room_for_the_nul() {
        let f: [c_char; 4] = cname("abcdef");
        assert_eq!(field_bytes(&f), b"abc\0");
        let exact: [c_char; 4] = cname("abcd");
        assert_eq!(field_bytes(&exact), b"abc\0", "a name as long as the field loses its last byte to the NUL");
    }

    #[test]
    fn cname_of_the_empty_string_and_of_a_one_byte_field() {
        let f: [c_char; 3] = cname("");
        assert_eq!(field_bytes(&f), [0, 0, 0]);
        let one: [c_char; 1] = cname("xyz");
        assert_eq!(field_bytes(&one), [0]);
        let none: [c_char; 0] = cname("xyz");
        assert!(none.is_empty());
    }

    #[test]
    fn set_name_copies_and_clears_the_rest_of_the_field() {
        let mut f = [b'#' as c_char; 8];
        set_name(&mut f, b"uv");
        assert_eq!(field_bytes(&f), b"uv\0\0\0\0\0\0", "stale bytes after the name are zeroed");
    }

    #[test]
    fn set_name_truncates_and_always_terminates() {
        let mut f = [b'#' as c_char; 4];
        set_name(&mut f, b"abcdefgh");
        assert_eq!(field_bytes(&f), b"abc\0");
        set_name(&mut f, b"abcd");
        assert_eq!(field_bytes(&f), b"abc\0");
        set_name(&mut f, b"abc");
        assert_eq!(field_bytes(&f), b"abc\0");
    }

    #[test]
    fn set_name_handles_empty_source_and_empty_field() {
        let mut f = [b'#' as c_char; 3];
        set_name(&mut f, b"");
        assert_eq!(field_bytes(&f), [0, 0, 0]);
        let mut empty: [c_char; 0] = [];
        set_name(&mut empty, b"abc"); /* must not panic */
        let mut one = [b'#' as c_char; 1];
        set_name(&mut one, b"abc");
        assert_eq!(field_bytes(&one), [0]);
    }

    #[test]
    fn name_bytes_stops_at_the_first_nul() {
        let f: [c_char; 8] = cname("ab");
        assert_eq!(name_bytes(&f), b"ab");
        let mut g = [0 as c_char; 6];
        set_name(&mut g, b"a");
        g[3] = b'z' as c_char; /* garbage after the terminator is not part of the name */
        assert_eq!(name_bytes(&g), b"a");
    }

    #[test]
    fn name_bytes_of_an_unterminated_field_is_the_whole_field() {
        let f = [b'x' as c_char; 4];
        assert_eq!(name_bytes(&f), b"xxxx");
        assert_eq!(name_bytes(&[]), b"");
        assert_eq!(name_bytes(&[0 as c_char; 4]), b"");
    }

    #[test]
    fn set_name_then_name_bytes_round_trips_high_bytes() {
        let mut f = [0 as c_char; 8];
        set_name(&mut f, &[0xC3, 0xA9, 0xFF]);
        assert_eq!(name_bytes(&f), &[0xC3, 0xA9, 0xFF]);
    }

    #[test]
    fn cstr_len_and_bytes() {
        // SAFETY: NUL-terminated literals; null is allowed by cstr_bytes.
        unsafe {
            assert_eq!(cstr_len(c"".as_ptr()), 0);
            assert_eq!(cstr_len(c"hello".as_ptr()), 5);
            assert_eq!(cstr_bytes(c"u_color".as_ptr()), b"u_color");
            assert_eq!(cstr_bytes(c"".as_ptr()), b"");
            assert_eq!(cstr_bytes(core::ptr::null()), b"");
        }
    }

    #[test]
    fn copy_out_str_fits() {
        let mut buf = [0x55 as c_char; 8];
        let mut len = -1;
        // SAFETY: buf holds the 8 bytes named; len is valid.
        unsafe { copy_out_str(b"abc", 8, &mut len, buf.as_mut_ptr()) };
        assert_eq!(len, 3);
        assert_eq!(&field_bytes(&buf)[..4], b"abc\0");
        assert_eq!(buf[4], 0x55, "nothing past the NUL is touched");
    }

    #[test]
    fn copy_out_str_truncates_to_buf_size_minus_one() {
        let mut buf = [0x55 as c_char; 8];
        let mut len = -1;
        // SAFETY: only the first 4 bytes of buf are offered.
        unsafe { copy_out_str(b"abcdef", 4, &mut len, buf.as_mut_ptr()) };
        assert_eq!(len, 3);
        assert_eq!(&field_bytes(&buf)[..5], b"abc\0\x55");
        // SAFETY: as above, exact fit needs buf_size = len + 1.
        unsafe { copy_out_str(b"abc", 3, &mut len, buf.as_mut_ptr()) };
        assert_eq!(len, 2);
        assert_eq!(&field_bytes(&buf)[..3], b"ab\0");
    }

    #[test]
    fn copy_out_str_with_room_for_the_nul_only() {
        let mut buf = [0x55 as c_char; 2];
        let mut len = -1;
        // SAFETY: one byte of buf is offered.
        unsafe { copy_out_str(b"abc", 1, &mut len, buf.as_mut_ptr()) };
        assert_eq!(len, 0);
        assert_eq!(field_bytes(&buf), [0, 0x55]);
    }

    #[test]
    fn copy_out_str_without_a_buffer_reports_zero() {
        let mut buf = [0x55 as c_char; 4];
        let mut len = -1;
        // SAFETY: null / zero-sized / negative-sized buffers are never written.
        unsafe {
            copy_out_str(b"abc", 8, &mut len, core::ptr::null_mut());
            assert_eq!(len, 0);
            len = -1;
            copy_out_str(b"abc", 0, &mut len, buf.as_mut_ptr());
            assert_eq!(len, 0);
            len = -1;
            copy_out_str(b"abc", -5, &mut len, buf.as_mut_ptr());
            assert_eq!(len, 0);
        }
        assert_eq!(field_bytes(&buf), [0x55; 4]);
    }

    #[test]
    fn copy_out_str_accepts_a_null_length() {
        let mut buf = [0x55 as c_char; 4];
        // SAFETY: buf holds 4 bytes; a null length is allowed.
        unsafe { copy_out_str(b"xy", 4, core::ptr::null_mut(), buf.as_mut_ptr()) };
        assert_eq!(field_bytes(&buf), b"xy\0\x55");
    }

    #[test]
    fn fmtbuf_formats_and_terminates() {
        let mut b = FmtBuf::<32>::new();
        assert_eq!(b.as_bytes(), b"");
        assert_eq!(b.as_cstr().to_bytes(), b"");
        write!(b, "x={} y=0x{:x}", 12, 255).unwrap();
        assert_eq!(b.as_bytes(), b"x=12 y=0xff");
        assert_eq!(b.as_cstr().to_bytes(), b"x=12 y=0xff");
        // SAFETY: as_cstr_ptr is NUL-terminated by construction.
        assert_eq!(unsafe { cstr_bytes(b.as_cstr_ptr()) }, b"x=12 y=0xff");
    }

    #[test]
    fn fmtbuf_appends_across_writes() {
        let mut b = FmtBuf::<16>::new();
        b.write_str("ab").unwrap();
        b.write_str("").unwrap();
        b.write_str("cd").unwrap();
        assert_eq!(b.as_bytes(), b"abcd");
        assert_eq!(b.as_cstr().to_bytes(), b"abcd");
    }

    #[test]
    fn fmtbuf_truncates_silently_at_n_minus_one() {
        let mut b = FmtBuf::<8>::new();
        assert!(write!(b, "0123456789").is_ok(), "truncation is not an error");
        assert_eq!(b.as_bytes(), b"0123456");
        assert_eq!(b.as_cstr().to_bytes(), b"0123456");
        assert!(b.write_str("more").is_ok());
        assert_eq!(b.as_bytes(), b"0123456", "a full buffer ignores further writes");
    }

    #[test]
    fn fmtbuf_truncates_in_the_middle_of_a_later_write() {
        let mut b = FmtBuf::<8>::new();
        b.write_str("01234").unwrap();
        b.write_str("56789").unwrap();
        assert_eq!(b.as_bytes(), b"0123456");
    }

    #[test]
    fn fmtbuf_of_one_byte_holds_only_the_nul() {
        let mut b = FmtBuf::<1>::new();
        b.write_str("abc").unwrap();
        assert_eq!(b.as_bytes(), b"");
        assert_eq!(b.as_cstr().to_bytes(), b"");
    }

    #[test]
    fn fmtbuf_cstr_view_stops_at_an_embedded_nul() {
        let mut b = FmtBuf::<16>::new();
        b.write_str("ab\0cd").unwrap();
        assert_eq!(b.as_bytes(), b"ab\0cd");
        assert_eq!(b.as_cstr().to_bytes(), b"ab");
    }

    #[test]
    fn cstr_writer_clears_the_buffer_and_appends() {
        let mut buf = [0x55 as c_char; 16];
        // SAFETY: buf outlives the writer and holds the 16 bytes named.
        let mut w = unsafe { CStrWriter::new(buf.as_mut_ptr(), 16) };
        // SAFETY: NUL-terminated by new().
        assert_eq!(unsafe { cstr_bytes(buf.as_ptr()) }, b"", "new() leaves an empty string");
        write!(w, "err {}", 3).unwrap();
        w.write_str("; next").unwrap();
        // SAFETY: NUL-terminated by the writer.
        assert_eq!(unsafe { cstr_bytes(buf.as_ptr()) }, b"err 3; next");
    }

    #[test]
    fn cstr_writer_truncates_and_terminates_inside_cap() {
        let mut buf = [0x55 as c_char; 8];
        // SAFETY: only the first 6 bytes are offered.
        let mut w = unsafe { CStrWriter::new(buf.as_mut_ptr(), 6) };
        assert!(w.write_str("abc").is_ok());
        assert!(w.write_str("defgh").is_ok());
        assert!(w.write_str("ij").is_ok());
        assert_eq!(field_bytes(&buf), b"abcde\0\x55\x55");
    }

    #[test]
    fn cstr_writer_with_cap_one_writes_only_the_nul() {
        let mut buf = [0x55 as c_char; 2];
        // SAFETY: one byte is offered.
        let mut w = unsafe { CStrWriter::new(buf.as_mut_ptr(), 1) };
        w.write_str("abc").unwrap();
        assert_eq!(field_bytes(&buf), [0, 0x55]);
    }

    #[test]
    fn cstr_writer_ignores_null_and_zero_capacity() {
        let mut buf = [0x55 as c_char; 4];
        // SAFETY: a null buffer and a zero capacity are never written.
        unsafe {
            let mut w = CStrWriter::new(core::ptr::null_mut(), 16);
            assert!(w.write_str("abc").is_ok());
            let mut z = CStrWriter::new(buf.as_mut_ptr(), 0);
            assert!(z.write_str("abc").is_ok());
        }
        assert_eq!(field_bytes(&buf), [0x55; 4]);
    }

    #[test]
    fn bytes_displays_ascii_and_masks_the_rest() {
        assert_eq!(format!("{}", Bytes(b"a_pos[0]")), "a_pos[0]");
        assert_eq!(format!("{}", Bytes(b"")), "");
        assert_eq!(format!("{}", Bytes(&[b'a', 0, b'b', 0x80, 0xFF, 0x7F])), "a?b??\x7f");
    }

    #[test]
    fn bytes_prints_valid_utf8_verbatim_and_masks_invalid_bytes() {
        assert_eq!(format!("<{}>", Bytes("é".as_bytes())), "<é>");
        assert_eq!(format!("{}", Bytes("// 頂点".as_bytes())), "// 頂点");
        /* a sequence cut short (a line truncated inside a character) and a stray continuation byte */
        assert_eq!(format!("{}", Bytes(&[b'x', 0xE9, 0xA0, b'y', 0x80])), "x??y?", "one ? per byte");
    }

    #[test]
    fn fmtbuf_needs_room_for_the_nul() {
        /* FmtBuf::<0>::new() does not compile (HAS_ROOM_FOR_THE_NUL); the smallest buffer works */
        let mut b = FmtBuf::<1>::default();
        assert!(write!(b, "{}", 12345).is_ok());
        // SAFETY: NUL-terminated by construction.
        assert_eq!(unsafe { cstr_len(b.as_cstr_ptr()) }, 0);
    }

    #[test]
    fn app_pointer_helpers_work_on_misaligned_memory() {
        let mut bytes = [0u8; 13];
        let p = bytes.as_mut_ptr().wrapping_add(1); /* odd address: misaligned for f32 */
        // SAFETY: 12 bytes from p are inside `bytes`.
        unsafe {
            app_set(p as *mut f32, 0, 1.5);
            app_set(p as *mut f32, 2, -2.0);
            assert_eq!(app_get(p as *const f32, 0), 1.5);
            assert_eq!(app_get(p as *const f32, 2), -2.0);
            app_put(p.wrapping_add(4) as *mut i32, 7);
            assert_eq!(app_get(p as *const i32, 1), 7);
            app_put(core::ptr::null_mut::<i32>(), 7); /* null: nothing */
        }
        assert_eq!(bytes[0], 0);
    }

    #[test]
    fn cbuf_starts_empty() {
        let b = CBuf::new();
        assert!(b.as_ptr().is_null());
        assert_eq!(b.capacity(), 0);
        let d = CBuf::default();
        assert!(d.as_ptr().is_null());
        assert_eq!(d.capacity(), 0);
    }

    #[test]
    fn cbuf_reserve_zero_on_an_empty_buffer_allocates_nothing() {
        let mut b = CBuf::new();
        assert!(b.reserve(0).is_null());
        assert_eq!(b.capacity(), 0);
    }

    #[test]
    fn cbuf_grows_only_when_asked_for_more() {
        let mut b = CBuf::new();
        let p = b.reserve(64);
        assert!(!p.is_null());
        assert_eq!(b.capacity(), 64);
        assert_eq!(b.as_ptr(), p);
        // SAFETY: 64 writable bytes were reserved.
        unsafe { core::ptr::write_bytes(p, 0xAB, 64) };

        let q = b.reserve(16);
        assert_eq!(q, p, "a smaller request keeps the block");
        assert_eq!(b.capacity(), 64);
        let q = b.reserve(64);
        assert_eq!(q, p, "an equal request keeps the block");
        // SAFETY: the block is still live.
        assert_eq!(unsafe { *q.add(63) }, 0xAB);

        let r = b.reserve(4096);
        assert!(!r.is_null());
        assert_eq!(b.capacity(), 4096);
        // SAFETY: 4096 writable bytes were reserved.
        unsafe { core::ptr::write_bytes(r, 0xCD, 4096) };
        b.release();
    }

    #[test]
    fn cbuf_release_resets_and_can_be_reused() {
        let mut b = CBuf::new();
        b.reserve(32);
        b.release();
        assert!(b.as_ptr().is_null());
        assert_eq!(b.capacity(), 0);
        b.release(); /* releasing twice is harmless */
        assert!(!b.reserve(8).is_null());
        assert_eq!(b.capacity(), 8);
        b.release();
    }

    #[test]
    fn global_new_holds_its_value_and_zeroed_is_zero() {
        static G: Global<u32> = Global::new(7);
        static Z: Global<[u16; 4]> = Global::zeroed();
        // SAFETY: these cells are private to this test.
        unsafe {
            assert_eq!(*G.get(), 7);
            *G.get() += 1;
            assert_eq!(*G.as_ptr(), 8);
            assert_eq!(*Z.get(), [0; 4]);
        }
    }
}
