// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The end of a glmark2 run (driver/samples/zephyr/glmark2), worded so a run
//! read from the record screen alone (the NanoPC-T6) says per entry of the
//! benchmark list whether it ran and why not (driver/docs/glmark2.md, "The
//! list at the end"). The sample keeps each entry's upstream info string, its
//! result and glmark2's first error lines; [`block_line`] words the block it
//! prints after the score banner, and the list part alone for a run that ends
//! without it (a fatal error, a hang). The C side reaches it through
//! `ffi::board` (`yd_gm_list_block`).
//!
//! The block fits the record screen of the benchmark's window ([`WINDOW_W`]
//! x [`WINDOW_H`]): every line at most [`COLS`] characters, so none wraps, and
//! with the lines `board_finish` prints after it ([`BOARD_END_LINES`]) at most
//! [`ROWS`] lines. It is, in order:
//!
//! ```text
//! glmark2: score 119: 15 scenes ran, 17 failed, 0 unsupported (33 list entries)
//! glmark2: 800x600, shown yes, GPU resets 0, GL errors recorded 1
//! glmark2: GL errors raised 352, draws that sampled an incomplete texture 0
//! glmark2-list:  0 build:use-vbo=false     ran 119 fps      1 build:use-vbo=true      ran 120 fps
//! glmark2-list:  2 texture:texture-filte.. failed link      3 texture:texture-filte.. failed link
//! ...
//! glmark2-list: why 2-4: link: a texture coordinate other than a vec2 varying read whole or two cons..
//! ```
//!
//! two entries a line (the index is the entry's identity, the name a help cut
//! to its cell), then each distinct failure reason once with the entries that
//! share it, as many as the rows left allow.

use super::record::{
    glmark2_why_name, GLMARK2_WHY_FILE, GLMARK2_WHY_FRAGMENT_SHADER, GLMARK2_WHY_LINK, GLMARK2_WHY_NO_ERROR,
    GLMARK2_WHY_VERTEX_SHADER,
};
use super::record_screen;

/// The entry's set-up succeeded and its frames ran.
pub const RAN: u32 = 0;
/// Its set-up failed (`why` and glmark2's error lines say why).
pub const FAILED: u32 = 1;
/// glmark2 says the scene is unsupported (an extension it needs is not claimed).
pub const UNSUPPORTED: u32 = 2;
/// The entry started and the run ended inside it (a fatal error, a hang).
pub const NO_END: u32 = 3;

/// The benchmark's window: CONFIG_GLMARK2_SIZE's default (the sample's
/// Kconfig; driver/docs/glmark2.md, "Why 800x600 on every board"). The record
/// screen is drawn over it.
pub const WINDOW_W: usize = 800;
pub const WINDOW_H: usize = 600;
/// Characters of a line at most: one row of the record screen over the window.
pub const COLS: usize = record_screen::cols(WINDOW_W);
/// Lines from the score line to `board_finish`'s last at most: the record
/// screen's rows for console lines over the window.
pub const ROWS: usize = record_screen::console_rows(WINDOW_H);
/// The lines `board_finish` prints after the block at most
/// (driver/zephyr/lib/board/common/board.c, board_finish and
/// board_hold_and_reset; the T6's board_reset): the sample's record, an APP
/// record when the GPU was reset, "shaderc: keep", "yakogl: draws refused",
/// the PASS or FAIL line, "board: holding", "board: screen state".
pub const BOARD_END_LINES: usize = 7;
/// Lines of the block at most.
pub const BLOCK_ROWS: usize = ROWS - BOARD_END_LINES;
/// The end lines before the list (score, shown, GL errors) and the pass line
/// after it.
const END_LINES: usize = 4;
/// Lines of the list part at most: the entries and the reasons.
pub const LIST_ROWS: usize = BLOCK_ROWS - END_LINES;

const _: () = assert!(COLS >= record_screen::RECORD_LINE, "the record takes one row");
const _: () = assert!(COLS <= record_screen::COLS, "the ring keeps a whole line");
const _: () = assert!(ROWS < record_screen::LINES, "the ring keeps every line of the block");
const _: () = assert!(LIST_ROWS >= 3, "an entry row, a note and a reason");

const PREFIX: &[u8] = b"glmark2-list: ";
/// One entry's cell: two a line, a space between.
const CELL: usize = (COLS - PREFIX.len() - 1) / 2;

/// One entry as the sample kept it.
#[derive(Clone, Copy, Debug)]
pub struct Scene<'a> {
    /// Upstream's `Scene::info_string()`: `[name] <opt=val:opt=val>`, or
    /// `[name] <default>` when no option is set.
    pub info: &'a [u8],
    /// glmark2's first error line while the entry set up and the line after
    /// it, `\n` between them; empty when there was none.
    pub error: &'a [u8],
    /// The index in glmark2's benchmark list.
    pub entry: u32,
    /// [`RAN`], [`FAILED`], [`UNSUPPORTED`] or [`NO_END`].
    pub result: u32,
    /// [`RAN`]: the FPS integer of upstream's result line.
    pub fps: u32,
    /// [`FAILED`]: the GLMARK2_WHY_* of the first error line.
    pub why: u32,
}

/// The run's own lines around the list: `struct yd_gm_end`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct End {
    pub score: u32,
    pub ran: u32,
    pub failed: u32,
    pub unsupported: u32,
    /// The benchmark list's entries.
    pub listed: u32,
    pub w: u32,
    pub h: u32,
    /// 1: `--off-screen`.
    pub offscreen: u32,
    /// 1: the window was shown.
    pub shown: u32,
    pub gpu_resets: u32,
    /// The GL core's counters (yakogl_gl_errors_raised, _recorded,
    /// yakogl_incomplete_texture_draws).
    pub gl_errors: u32,
    pub gl_recorded: u32,
    pub incomplete_draws: u32,
    /// 1: the run passes (no scene failed, the score is not 0).
    pub pass: u32,
}

/// The scene's name and its options (`opt=val:...`, empty for `<default>`).
fn parse(info: &[u8]) -> (&[u8], &[u8]) {
    let Some(rest) = info.strip_prefix(b"[") else { return (info, &[]) };
    let Some(close) = rest.iter().position(|&c| c == b']') else { return (info, &[]) };
    let opts = rest[close + 1..].strip_prefix(b" <").and_then(|o| o.strip_suffix(b">")).unwrap_or(&[]);
    (&rest[..close], if opts == b"default" { &[] } else { opts })
}

fn options(opts: &[u8]) -> impl Iterator<Item = &[u8]> {
    opts.split(|&c| c == b':').filter(|o| !o.is_empty())
}

fn key(opt: &[u8]) -> &[u8] {
    opt.split(|&c| c == b'=').next().unwrap_or(opt)
}

/// Which of entry `i`'s options (bit k: its k-th, at most 64) its name keeps:
/// the fewest that tell it apart from every other entry of the same scene in
/// the list, those of an option every such entry sets first (`effect=blur`
/// before `blur-radius=5`), never one they all share. None when the scene has
/// one entry.
fn kept(list: &[Scene], i: usize) -> u64 {
    let (scene, opts) = parse(list[i].info);
    let others = || {
        list.iter().enumerate().filter(move |&(j, s)| j != i && parse(s.info).0 == scene).map(|(_, s)| parse(s.info).1)
    };
    let has = |o: &[u8], opt: &[u8]| options(o).any(|x| x == opt);
    let apart = |mask: u64| {
        others().all(|o| options(opts).take(64).enumerate().any(|(k, opt)| mask & 1 << k != 0 && !has(o, opt)))
    };
    let mut mask = 0u64;
    for axis_first in [true, false] {
        for (k, opt) in options(opts).take(64).enumerate() {
            if apart(mask) {
                return mask;
            }
            let axis = others().all(|o| options(o).any(|x| key(x) == key(opt)));
            if axis == axis_first && !others().all(|o| has(o, opt)) {
                mask |= 1 << k;
            }
        }
    }
    mask
}

/// A line under construction: never longer than [`COLS`], every byte
/// printable ASCII.
struct Line<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl Line<'_> {
    fn room(&self) -> usize {
        self.out.len().min(COLS) - self.len
    }

    fn put(&mut self, s: &[u8]) {
        for &c in s.iter().take(self.room()) {
            self.out[self.len] = if (0x20..0x7f).contains(&c) { c } else { b'?' };
            self.len += 1;
        }
    }

    fn num(&mut self, n: u32, width: usize) {
        let mut d = [b' '; 10];
        let mut i = d.len();
        let mut n = n;
        loop {
            i -= 1;
            d[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let start = i.min(d.len().saturating_sub(width));
        self.put(&d[start..]);
    }

    fn pad(&mut self, to: usize) {
        while self.len < to && self.room() != 0 {
            self.put(b" ");
        }
    }

    /// `s` whole when it fits `max`, else its first characters and "..".
    fn fit(&mut self, s: &[u8], max: usize) {
        if s.len() <= max {
            self.put(s);
        } else if max >= 2 {
            self.put(&s[..max - 2]);
            self.put(b"..");
        }
    }
}

fn digits(n: u32) -> usize {
    n.checked_ilog10().unwrap_or(0) as usize + 1
}

/// The name, short: the scene's and the options [`kept`] keeps, `:` between,
/// cut to `max`.
fn name(list: &[Scene], i: usize, l: &mut Line, max: usize) {
    let (scene, opts) = parse(list[i].info);
    let mask = kept(list, i);
    let mut buf = [0u8; COLS];
    let mut n = Line { out: &mut buf, len: 0 };
    n.put(scene);
    for (_, opt) in options(opts).take(64).enumerate().filter(|&(k, _)| mask & 1 << k != 0) {
        n.put(b":");
        n.put(opt);
    }
    let len = n.len;
    l.fit(&buf[..len], max);
}

/// The step in a cell: a word of the record's GLMARK2_WHY_* name.
fn step(why: u32) -> &'static [u8] {
    match why {
        GLMARK2_WHY_NO_ERROR => b"",
        GLMARK2_WHY_VERTEX_SHADER => b" vertex",
        GLMARK2_WHY_FRAGMENT_SHADER => b" fragment",
        GLMARK2_WHY_LINK => b" link",
        GLMARK2_WHY_FILE => b" file",
        _ => b" other",
    }
}

/// An entry's result in its cell.
fn result(s: &Scene, l: &mut Line) {
    match s.result {
        RAN => {
            l.put(b"ran ");
            l.num(s.fps, 0);
            l.put(b" fps");
        }
        FAILED => {
            l.put(b"failed");
            l.put(step(s.why));
        }
        UNSUPPORTED => l.put(b"unsupported"),
        NO_END => l.put(b"no end"),
        _ => l.put(b"unknown"),
    }
}

fn result_len(s: &Scene) -> usize {
    let mut buf = [0u8; COLS];
    let mut l = Line { out: &mut buf, len: 0 };
    result(s, &mut l);
    l.len
}

fn trim(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&c| c != b' ').unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c != b' ').map_or(start, |e| e + 1);
    &s[start..end.max(start)]
}

/// What glmark2's error says, without its own head: the second line of
/// "Failed to add ... shader from file F:" (the compiler's log), the text
/// after "Failed to link program ... files A and B:" (the linker's), or the
/// whole of any other error line.
fn reason(error: &[u8]) -> &[u8] {
    let mut lines = error.split(|&c| c == b'\n');
    let first = lines.next().unwrap_or(&[]);
    let first = trim(first.strip_prefix(b"Error: ").unwrap_or(first));
    if let Some(head) = first.strip_suffix(b":") {
        return match lines.next().and_then(|l| l.strip_prefix(b"Error: ")) {
            Some(log) if !trim(log).is_empty() => trim(log),
            _ => head,
        };
    }
    if first.starts_with(b"Failed to ") {
        if let Some(at) = first.windows(2).position(|w| w == b": ") {
            return trim(&first[at + 2..]);
        }
    }
    first
}

/// The last clause of a reason, after its last ": ", without the closing
/// brackets it does not open: what a nested log ("online shaderc: the code
/// generator refused this fragment shader: Mali(Valhall: <what>)") says last
/// and most exactly.
fn last_clause(r: &[u8]) -> &[u8] {
    let Some(at) = r.windows(2).rposition(|w| w == b": ") else { return r };
    let mut c = trim(&r[at + 2..]);
    let count = |s: &[u8], b: u8| s.iter().filter(|&&x| x == b).count();
    while c.last() == Some(&b')') && count(c, b')') > count(c, b'(') {
        c = &c[..c.len() - 1];
    }
    c
}

/// Two failed entries share a reason: the same step and the same words.
fn same_reason(a: &Scene, b: &Scene) -> bool {
    a.result == FAILED && b.result == FAILED && a.why == b.why && reason(a.error) == reason(b.error)
}

/// Whether entry `i` is the first of its reason (a failed entry no earlier
/// one shares a reason with).
fn first_of_reason(list: &[Scene], i: usize) -> bool {
    list[i].result == FAILED && !list[..i].iter().any(|s| same_reason(s, &list[i]))
}

/// The `g`-th distinct reason: the index of its first entry.
fn reason_at(list: &[Scene], g: usize) -> Option<usize> {
    (0..list.len()).filter(|&i| first_of_reason(list, i)).nth(g)
}

/// "why 2-4 7: link: <reason>": the entries that share entry `i`'s reason
/// (runs of three or more as a range), its step and its words, the words cut
/// to the line.
fn reason_line(list: &[Scene], i: usize, l: &mut Line) {
    l.put(b"why");
    let share: &dyn Fn(usize) -> bool = &|j: usize| same_reason(&list[j], &list[i]);
    let mut j = 0;
    while j < list.len() {
        if !share(j) {
            j += 1;
            continue;
        }
        let mut end = j;
        while end + 1 < list.len() && share(end + 1) && list[end + 1].entry == list[end].entry + 1 {
            end += 1;
        }
        l.put(b" ");
        l.num(list[j].entry, 0);
        if end >= j + 2 {
            l.put(b"-");
            l.num(list[end].entry, 0);
        } else if end == j + 1 {
            l.put(b" ");
            l.num(list[end].entry, 0);
        }
        j = end + 1;
    }
    l.put(b": ");
    l.put(glmark2_why_name(list[i].why).as_bytes());
    let r = reason(list[i].error);
    if list[i].why != GLMARK2_WHY_NO_ERROR && !r.is_empty() && l.room() > 2 {
        l.put(b": ");
        let room = l.room();
        l.fit(if r.len() <= room { r } else { last_clause(r) }, room);
    }
}

/// How the list part is laid out: the entries shown (the last ones when not
/// all fit), whether a note says how many are not, the entry rows, the
/// reasons shown, whether a note says how many are not.
struct Layout {
    first: usize,
    entries_note: bool,
    entry_rows: usize,
    reasons: usize,
    reasons_note: bool,
}

impl Layout {
    fn new(list: &[Scene]) -> Self {
        let mut first = 0;
        let mut entry_rows = list.len().div_ceil(2);
        let entries_note = entry_rows > LIST_ROWS - 1;
        if entries_note {
            // a note, the last entries, one row for the reasons' note
            entry_rows = LIST_ROWS - 2;
            first = list.len() - 2 * entry_rows;
        }
        let left = LIST_ROWS - entry_rows - usize::from(entries_note);
        let distinct = (0..list.len()).filter(|&i| first_of_reason(list, i)).count();
        let reasons_note = distinct > left;
        let reasons = if reasons_note { left - 1 } else { distinct };
        Layout { first, entries_note, entry_rows, reasons, reasons_note }
    }

    fn lines(&self) -> usize {
        usize::from(self.entries_note) + self.entry_rows + self.reasons + usize::from(self.reasons_note)
    }
}

/// Line `k` of the list part.
fn list_line(list: &[Scene], k: usize, l: &mut Line) {
    let lay = Layout::new(list);
    let shown = &list[lay.first..];
    let distinct = (0..list.len()).filter(|&i| first_of_reason(list, i)).count();
    let mut k = k;
    l.put(PREFIX);
    if lay.entries_note {
        if k == 0 {
            l.num(lay.first as u32, 0);
            l.put(b" earlier entries not shown");
            return;
        }
        k -= 1;
    }
    if k < lay.entry_rows {
        let iw = digits(shown.iter().map(|s| s.entry).max().unwrap_or(0));
        let rw = shown.iter().map(result_len).max().unwrap_or(0);
        let nw = CELL.saturating_sub(iw + 1 + 1 + rw);
        for c in 0..2 {
            let Some(s) = shown.get(2 * k + c) else { break };
            let at = l.len;
            if c == 1 {
                l.put(b" ");
            }
            l.num(s.entry, iw);
            l.put(b" ");
            let n = l.len;
            name(list, lay.first + 2 * k + c, l, nw);
            l.pad(n + nw);
            l.put(b" ");
            result(s, l);
            if c == 0 && shown.get(2 * k + 1).is_some() {
                l.pad(at + CELL);
            }
        }
        return;
    }
    k -= lay.entry_rows;
    if k < lay.reasons {
        if let Some(i) = reason_at(list, k) {
            reason_line(list, i, l);
        }
        return;
    }
    l.num((distinct - lay.reasons) as u32, 0);
    l.put(b" more reasons not shown");
}

/// Line `k` of the end lines before the list.
fn end_line(e: &End, k: usize, l: &mut Line) {
    match k {
        0 => {
            l.put(b"glmark2: score ");
            l.num(e.score, 0);
            l.put(b": ");
            l.num(e.ran, 0);
            l.put(b" scenes ran, ");
            l.num(e.failed, 0);
            l.put(b" failed, ");
            l.num(e.unsupported, 0);
            l.put(b" unsupported (");
            l.num(e.listed, 0);
            l.put(b" list entries)");
        }
        1 => {
            l.put(b"glmark2: ");
            l.num(e.w, 0);
            l.put(b"x");
            l.num(e.h, 0);
            if e.offscreen != 0 {
                l.put(b" off-screen");
            }
            l.put(if e.shown != 0 { b", shown yes" } else { b", shown no" });
            l.put(b", GPU resets ");
            l.num(e.gpu_resets, 0);
            l.put(b", GL errors recorded ");
            l.num(e.gl_recorded, 0);
        }
        _ => {
            l.put(b"glmark2: GL errors raised ");
            l.num(e.gl_errors, 0);
            l.put(b", draws that sampled an incomplete texture ");
            l.num(e.incomplete_draws, 0);
        }
    }
}

/// Line `k` of the block into `out`: its length, at most [`COLS`] and
/// `out.len()`; no newline; 0 past the block's last line (no line is empty).
/// With `end` the whole block (the end lines, the list part, a pass's
/// unsupported line), without it the list part alone. The block has at most
/// [`BLOCK_ROWS`] lines, the list part [`LIST_ROWS`].
pub fn block_line(end: Option<&End>, list: &[Scene], k: usize, out: &mut [u8]) -> usize {
    let mut l = Line { out, len: 0 };
    let mut k = k;
    if let Some(e) = end {
        if k < 3 {
            end_line(e, k, &mut l);
            return l.len;
        }
        k -= 3;
    }
    let n = Layout::new(list).lines();
    if k < n {
        list_line(list, k, &mut l);
        return l.len;
    }
    match end {
        Some(e) if k == n && e.pass != 0 && e.unsupported != 0 => {
            l.put(b"glmark2: no scene failed; ");
            l.num(e.unsupported, 0);
            l.put(b" unsupported, left out of the score (not a pass or a fail)");
        }
        _ => {}
    }
    l.len
}

#[cfg(test)]
mod tests {
    use super::super::record::GLMARK2_WHY_OTHER;
    use super::*;

    /// glmark2's default list (default-benchmarks.h at the pinned commit) as
    /// upstream's info strings name its entries: the options set, by name.
    const DEFAULT_LIST: [&str; 33] = [
        "[build] <use-vbo=false>",
        "[build] <use-vbo=true>",
        "[texture] <texture-filter=nearest>",
        "[texture] <texture-filter=linear>",
        "[texture] <texture-filter=mipmap>",
        "[shading] <shading=gouraud>",
        "[shading] <shading=blinn-phong-inf>",
        "[shading] <shading=phong>",
        "[shading] <shading=cel>",
        "[bump] <bump-render=high-poly>",
        "[bump] <bump-render=normals>",
        "[bump] <bump-render=height>",
        "[effect2d] <kernel=0,1,0;1,-4,1;0,1,0;>",
        "[effect2d] <kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;>",
        "[pulsar] <light=false:quads=5:texture=false>",
        "[desktop] <blur-radius=5:effect=blur:passes=1:separable=true:windows=4>",
        "[desktop] <effect=shadow:windows=4>",
        "[buffer] <columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map>",
        "[buffer] <columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata>",
        "[buffer] <columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map>",
        "[ideas] <speed=duration>",
        "[jellyfish] <default>",
        "[terrain] <default>",
        "[shadow] <default>",
        "[refract] <default>",
        "[conditionals] <fragment-steps=0:vertex-steps=0>",
        "[conditionals] <fragment-steps=5:vertex-steps=0>",
        "[conditionals] <fragment-steps=0:vertex-steps=5>",
        "[function] <fragment-complexity=low:fragment-steps=5>",
        "[function] <fragment-complexity=medium:fragment-steps=5>",
        "[loop] <fragment-loop=false:fragment-steps=5:vertex-steps=5>",
        "[loop] <fragment-steps=5:fragment-uniform=false:vertex-steps=5>",
        "[loop] <fragment-steps=5:fragment-uniform=true:vertex-steps=5>",
    ];

    /// The texture scene's link error of the NanoPC-T6 run of 2026-11-23,
    /// whole (the record screen cut it at its 158th character).
    const LINK_ERROR: &str = "Error: Failed to link program created from files None and None:  link error: the \
        vertex shader for the program's fragment shader: online shaderc: the code generator refused this \
        fragment shader: Mali(Valhall: a texture coordinate other than a vec2 varying read whole or two \
        constants is not lowered yet)";

    /// The fragment shaders the Mali code generator refuses in the default
    /// list (tools/hostharness/glmark2/FINDINGS-mali-allscenes.md), as
    /// glmark2 logs them.
    fn mali_fs(what: &str) -> &'static [u8] {
        let s = format!(
            "Error: Failed to add fragment shader from file None:\nError:   online shaderc: the code generator \
             refused this fragment shader: Mali(Valhall: {what})"
        );
        Box::leak(s.into_boxed_str()).as_bytes()
    }

    fn scene(info: &'static str, entry: u32, result: u32) -> Scene<'static> {
        Scene { info: info.as_bytes(), error: b"", entry, result, fps: 0, why: GLMARK2_WHY_NO_ERROR }
    }

    fn block(end: Option<&End>, list: &[Scene]) -> Vec<String> {
        let mut v = Vec::new();
        loop {
            let mut out = [0u8; 256];
            let n = block_line(end, list, v.len(), &mut out);
            if n == 0 {
                return v;
            }
            v.push(String::from_utf8(out[..n].to_vec()).unwrap());
        }
    }

    /// The default list as the Mali runs it today (the host harness's
    /// record): 14 ran, 17 failed at a refused shader, terrain the run's end.
    fn mali_run() -> Vec<Scene<'static>> {
        let const_fold = "a constant operation on or to a value that is not a finite normal float is not lowered yet";
        let arith_const = "a constant product in a shape Mesa's algebraic rules move or fold is not lowered yet";
        let tex_multi = "a fragment shader texture look-up beside another in a form only a lone look-up takes \
                         (a mediump computed coordinate, a cube map), or more than 64 look-ups, is not lowered yet";
        DEFAULT_LIST
            .iter()
            .enumerate()
            .map(|(i, info)| {
                let mut s = scene(info, i as u32, RAN);
                s.fps = 119 + (i as u32 % 3);
                let fail = |s: &mut Scene<'static>, why, error| (s.result, s.why, s.error) = (FAILED, why, error);
                match i {
                    2..=4 => fail(&mut s, GLMARK2_WHY_LINK, LINK_ERROR.as_bytes()),
                    6 | 8..=11 => fail(&mut s, GLMARK2_WHY_FRAGMENT_SHADER, mali_fs(const_fold)),
                    7 | 24 => fail(&mut s, GLMARK2_WHY_FRAGMENT_SHADER, mali_fs(arith_const)),
                    12 | 13 => fail(&mut s, GLMARK2_WHY_FRAGMENT_SHADER, mali_fs(tex_multi)),
                    20 => fail(
                        &mut s,
                        GLMARK2_WHY_VERTEX_SHADER,
                        b"Error: Failed to add vertex shader from file None:\nError:   online shaderc: the code \
                          generator refused this vertex shader: Mali(Valhall: control flow (if, a loop, a return \
                          before the end) is not lowered yet)",
                    ),
                    21 | 23 | 29 | 32 => fail(&mut s, GLMARK2_WHY_FRAGMENT_SHADER, mali_fs("arithmetic")),
                    22 => s.result = NO_END,
                    _ => {}
                }
                s
            })
            .collect()
    }

    fn end_of(list: &[Scene]) -> End {
        let count = |r| list.iter().filter(|s| s.result == r).count() as u32;
        End {
            score: 119,
            ran: count(RAN),
            failed: count(FAILED),
            unsupported: count(UNSUPPORTED),
            listed: 33,
            w: 800,
            h: 600,
            shown: 1,
            gl_errors: 352,
            gl_recorded: 1,
            ..End::default()
        }
    }

    #[test]
    fn the_mali_run_of_the_default_list_reads_on_one_screen() {
        let list = mali_run();
        let b = block(Some(&end_of(&list)), &list);
        // printed for the record of this module's output (cargo test -- --nocapture)
        for x in &b {
            println!("{x}");
        }
        assert_eq!(b[0], "glmark2: score 119: 15 scenes ran, 17 failed, 0 unsupported (33 list entries)");
        assert_eq!(b[1], "glmark2: 800x600, shown yes, GPU resets 0, GL errors recorded 1");
        assert_eq!(
            b[3],
            "glmark2-list:  0 build:use-vbo=false     ran 119 fps      1 build:use-vbo=true      ran 120 fps"
        );
        assert_eq!(
            b[4],
            "glmark2-list:  2 texture:texture-filte.. failed link      3 texture:texture-filte.. failed link"
        );
        assert_eq!(
            b[14],
            "glmark2-list: 22 terrain                 no end          23 shadow                  failed fragment"
        );
        assert_eq!(b[19], "glmark2-list: 32 loop:fragment-uniform.. failed fragment");
        assert_eq!(
            b[20],
            "glmark2-list: why 2-4: link: a texture coordinate other than a vec2 varying read whole or two cons.."
        );
        assert_eq!(
            b[21],
            "glmark2-list: why 6 8-11: fragment shader: a constant operation on or to a value that is not a fin.."
        );
        assert_eq!(b[25], "glmark2-list: why 21 23 29 32: fragment shader: arithmetic");
        assert_eq!(b.len(), 26, "every reason shown");
    }

    /// The worst run the block must fit: every entry failed with a reason of
    /// its own, or with the counters at their largest, with eight GL error
    /// lines before it and the most `board_finish` prints after it. Every line
    /// is one row of the record screen and the whole is on one screen and in
    /// the ring.
    #[test]
    fn the_worst_run_fits_the_record_screen() {
        let long_reason = |i: usize| mali_fs(&format!("reason {i} {}", "is long and not like any other ".repeat(5)));
        let all_failed: Vec<Scene> = DEFAULT_LIST
            .iter()
            .enumerate()
            .map(|(i, info)| {
                let mut s = scene(info, i as u32, FAILED);
                (s.why, s.error) = ([GLMARK2_WHY_FRAGMENT_SHADER, GLMARK2_WHY_OTHER][i % 2], long_reason(i));
                s
            })
            .collect();
        let mut mixed = all_failed.clone();
        mixed[17].result = UNSUPPORTED;
        mixed[19].result = UNSUPPORTED;
        mixed[32].result = NO_END;
        let mut ran = all_failed.clone();
        for s in &mut ran {
            (s.result, s.fps) = (RAN, u32::MAX);
        }
        ran[3].result = UNSUPPORTED;
        let max = End {
            score: u32::MAX,
            ran: 33,
            failed: 33,
            unsupported: 33,
            listed: 33,
            // the largest window the sample takes (main.cpp parse_size)
            w: 8192,
            h: 8192,
            offscreen: 1,
            shown: 1,
            gpu_resets: u32::MAX,
            gl_errors: u32::MAX,
            gl_recorded: u32::MAX,
            incomplete_draws: u32::MAX,
            pass: 0,
        };
        let pass = End { pass: 1, ..max };
        // what board_finish prints after the block, at its largest
        // (common/record.c, common/board.c board_finish and
        // board_hold_and_reset, nanopc_t6/board_nanopc_t6.c board_reset)
        let board_end = [
            "yk-record: ffffffff ffffffff ffffffff ffffffff".to_string(),
            "yk-record: ffffffff ffffffff ffffffff ffffffff".to_string(),
            format!("shaderc: keep high-water {} of {} bytes ({} held now)", usize::MAX, u32::MAX, usize::MAX),
            format!("yakogl: draws refused {}", u32::MAX),
            format!("glmark2: FAIL the GPU was reset {} time(s): a recovered hang, not a clean run", u32::MAX),
            format!("board: holding {} s, then the reset", i32::MAX),
            format!("board: screen state {}, {} flips", u32::MAX, u32::MAX),
        ];
        assert_eq!(board_end.len(), BOARD_END_LINES);
        for x in &board_end {
            assert!(x.len() <= COLS, "{x}");
        }
        for (end, list) in [(&max, &all_failed), (&max, &mixed), (&pass, &ran)] {
            let b = block(Some(end), list);
            for x in &b {
                assert!(x.len() <= COLS && x.bytes().all(|c| (0x20..0x7f).contains(&c)), "{x}");
            }
            // nothing a line holds was cut by the cap: the end lines end as worded
            assert!(b[0].ends_with("(33 list entries)") && b[1].ends_with(&u32::MAX.to_string()), "{b:?}");
            assert!(b[2].ends_with(&format!("texture {}", u32::MAX)), "{}", b[2]);
            assert!(b.len() <= BLOCK_ROWS, "{}", b.len());
            assert!(b.len() + BOARD_END_LINES <= ROWS);
            // eight GL error lines before the score line, all in the ring
            assert!(8 + b.len() + BOARD_END_LINES < record_screen::LINES);
            // all 33 entries are on it
            for i in 0..33 {
                let shown = b.iter().any(|x| {
                    x.starts_with("glmark2-list: ")
                        && !x.contains(" why ")
                        && x.split_whitespace().any(|w| w == i.to_string())
                });
                assert!(shown, "entry {i}: {b:?}");
            }
        }
        let b = block(Some(&max), &all_failed);
        assert_eq!(b.len(), BLOCK_ROWS - 1, "every list row used; a fail has no pass line");
        assert_eq!(b.last().unwrap(), "glmark2-list: 27 more reasons not shown");
        assert_eq!((COLS, ROWS, BLOCK_ROWS, LIST_ROWS), (100, 35, 28, 24));
        // the list part alone (a fatal error, a hang)
        let l = block(None, &mixed);
        assert_eq!(l.len(), LIST_ROWS);
        assert_eq!(&l[..], &block(Some(&max), &mixed)[3..3 + LIST_ROWS]);
        let p = block(Some(&pass), &ran);
        assert_eq!(
            p.last().unwrap(),
            &format!("glmark2: no scene failed; {} unsupported, left out of the score (not a pass or a fail)", 33)
        );
    }

    #[test]
    fn a_longer_list_shows_its_last_entries_and_says_so() {
        let mut list: Vec<Scene> = (0..100).map(|i| scene(DEFAULT_LIST[i % 33], i as u32, RAN)).collect();
        list[99].result = FAILED;
        list[99].why = GLMARK2_WHY_FILE;
        list[99].error = b"Error: Could not open 3ds file 'x'";
        let b = block(None, &list);
        assert_eq!(b.len(), LIST_ROWS);
        let shown = 2 * (LIST_ROWS - 2);
        assert_eq!(b[0], format!("glmark2-list: {} earlier entries not shown", 100 - shown));
        assert!(b[1].starts_with(&format!("glmark2-list: {} ", 100 - shown)), "{}", b[1]);
        assert_eq!(b[LIST_ROWS - 1], "glmark2-list: why 99: a file: Could not open 3ds file 'x'");
        assert!(b.iter().all(|x| x.len() <= COLS));
    }

    #[test]
    fn names_keep_the_options_that_tell_the_entries_of_a_scene_apart() {
        let list: Vec<_> = DEFAULT_LIST.iter().enumerate().map(|(i, info)| scene(info, i as u32, RAN)).collect();
        let names: Vec<String> = (0..list.len())
            .map(|i| {
                let mut out = [0u8; COLS];
                let mut l = Line { out: &mut out, len: 0 };
                name(&list, i, &mut l, COLS);
                let n = l.len;
                String::from_utf8(out[..n].to_vec()).unwrap()
            })
            .collect();
        let want = [
            "build:use-vbo=false",
            "build:use-vbo=true",
            "texture:texture-filter=nearest",
            "texture:texture-filter=linear",
            "texture:texture-filter=mipmap",
            "shading:shading=gouraud",
            "shading:shading=blinn-phong-inf",
            "shading:shading=phong",
            "shading:shading=cel",
            "bump:bump-render=high-poly",
            "bump:bump-render=normals",
            "bump:bump-render=height",
            "effect2d:kernel=0,1,0;1,-4,1;0,1,0;",
            "effect2d:kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1;",
            "pulsar",
            "desktop:effect=blur",
            "desktop:effect=shadow",
            "buffer:interleave=false:update-method=map",
            "buffer:interleave=false:update-method=subdata",
            "buffer:interleave=true",
            "ideas",
            "jellyfish",
            "terrain",
            "shadow",
            "refract",
            "conditionals:fragment-steps=0:vertex-steps=0",
            "conditionals:fragment-steps=5",
            "conditionals:fragment-steps=0:vertex-steps=5",
            "function:fragment-complexity=low",
            "function:fragment-complexity=medium",
            "loop:fragment-loop=false",
            "loop:fragment-uniform=false",
            "loop:fragment-uniform=true",
        ];
        assert_eq!(names, want);
        // the same entry twice: nothing tells them apart, the index does
        let twice = [scene("[build] <use-vbo=true>", 0, RAN), scene("[build] <use-vbo=true>", 1, RAN)];
        assert_eq!(
            block(None, &twice),
            ["glmark2-list: 0 build                          ran 0 fps 1 build                          ran 0 fps"]
        );
    }

    #[test]
    fn the_reason_is_glmark2_s_error_without_its_head() {
        assert!(reason(LINK_ERROR.as_bytes()).starts_with(b"link error: the vertex shader"));
        assert_eq!(
            last_clause(reason(LINK_ERROR.as_bytes())),
            b"a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet"
        );
        let shader = b"Error: Failed to add vertex shader from file None:\nError:   0:3: 'x' : undeclared identifier";
        assert_eq!(reason(shader), b"0:3: 'x' : undeclared identifier");
        assert_eq!(
            reason(b"Error: Failed to add vertex shader from file None:"),
            b"Failed to add vertex shader from file None"
        );
        assert_eq!(reason(b"Error: Could not open 3ds file 'horse.3ds'"), b"Could not open 3ds file 'horse.3ds'");
        assert_eq!(reason(b""), b"");
        // a bracket the clause opens stays
        assert_eq!(last_clause(b"a: f(x) and g(y))"), b"f(x) and g(y)");
        assert_eq!(last_clause(b"no clause"), b"no clause");
        // a failure without an error line, and bytes that are not printable
        let mut s = scene("[shadow] <default>", 23, FAILED);
        s.error = b"Error: ignored without a why";
        let mut t = scene("[refract] <default>", 24, FAILED);
        (t.why, t.error) = (GLMARK2_WHY_OTHER, b"Error: bad\x01 \xff byte");
        assert_eq!(
            block(None, &[s, t]),
            [
                "glmark2-list: 23 shadow                     failed       24 refract                    failed other",
                "glmark2-list: why 23: no error logged",
                "glmark2-list: why 24: another error: bad? ? byte",
            ]
        );
    }
}
