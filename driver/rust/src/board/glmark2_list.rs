// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The end summary of a glmark2 run (driver/samples/zephyr/glmark2): one
//! `glmark2-list:` line per benchmark-list entry that reached its set-up, so a
//! run read from the record screen alone (the NanoPC-T6) says which scenes ran
//! and why the others did not (driver/docs/glmark2.md, "The list at the
//! end"). The sample keeps each entry's upstream info string, its result and
//! glmark2's first error lines; [`line`] words one line from them. The C side
//! reaches it through `ffi::board` (`yd_gm_list_line`).
//!
//! A line: `glmark2-list: <entry> <name> <result>`, the entries and the names
//! padded to the longest in the list, at most [`LINE_MAX`] characters:
//!
//! ```text
//! glmark2-list:  0 build:use-vbo=false       ran 119 fps
//! glmark2-list:  2 texture:texture-filter=nearest failed, link: a texture coordinate ...
//! ```

use super::record::{glmark2_why_name, GLMARK2_WHY_NO_ERROR};
use super::record_screen;

/// The entry's set-up succeeded and its frames ran.
pub const RAN: u32 = 0;
/// Its set-up failed (`why` and glmark2's error lines say why).
pub const FAILED: u32 = 1;
/// glmark2 says the scene is unsupported (an extension it needs is not claimed).
pub const UNSUPPORTED: u32 = 2;
/// The entry started and the run ended inside it (a fatal error, a hang).
pub const NO_END: u32 = 3;

/// Characters of a line at most: one row of the record screen's widest
/// picture. The NanoPC-T6 shows a window of up to 1024 pixels, 128 characters
/// of the 8x16 font (driver/docs/nanopc-t6-display.md, "The record screen");
/// a longer line takes two rows there.
pub const LINE_MAX: usize = 128;
const _: () = assert!(LINE_MAX <= record_screen::COLS, "the ring keeps a whole line");
/// Characters of a name at most; a longer one ends in "..".
pub const NAME_MAX: usize = 48;

const PREFIX: &[u8] = b"glmark2-list: ";

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

/// A line under construction: never longer than [`LINE_MAX`], every byte
/// printable ASCII.
struct Line<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl Line<'_> {
    fn room(&self) -> usize {
        self.out.len().min(LINE_MAX) - self.len
    }

    fn put(&mut self, s: &[u8]) {
        for &c in s.iter().take(self.room()) {
            self.out[self.len] = if (0x20..0x7f).contains(&c) { c } else { b'?' };
            self.len += 1;
        }
    }

    fn num(&mut self, mut n: u32, width: usize) {
        let mut d = [b' '; 10];
        let mut i = d.len();
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

    /// `s` whole when it fits, else its first characters and "..".
    fn fit(&mut self, s: &[u8], max: usize) {
        if s.len() <= max {
            self.put(s);
        } else if max >= 2 {
            self.put(&s[..max - 2]);
            self.put(b"..");
        }
    }
}

/// The name, short: the scene's and the options [`kept`] keeps, `:` between.
fn name(list: &[Scene], i: usize, l: &mut Line) {
    let (scene, opts) = parse(list[i].info);
    let mask = kept(list, i);
    let mut buf = [0u8; LINE_MAX];
    let mut n = Line { out: &mut buf, len: 0 };
    n.put(scene);
    for (_, opt) in options(opts).take(64).enumerate().filter(|&(k, _)| mask & 1 << k != 0) {
        n.put(b":");
        n.put(opt);
    }
    let len = n.len;
    l.fit(&buf[..len], NAME_MAX);
}

fn name_len(list: &[Scene], i: usize) -> usize {
    let mut buf = [0u8; LINE_MAX];
    let mut l = Line { out: &mut buf, len: 0 };
    name(list, i, &mut l);
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

/// Line `i` of the list into `out`: its length, at most [`LINE_MAX`] and
/// `out.len()`; no newline. 0 when `i` is not in the list.
pub fn line(list: &[Scene], i: usize, out: &mut [u8]) -> usize {
    let Some(s) = list.get(i) else { return 0 };
    let entry_w = list.iter().map(|s| s.entry).max().unwrap_or(0).checked_ilog10().unwrap_or(0) as usize + 1;
    let name_w = (0..list.len()).map(|j| name_len(list, j)).max().unwrap_or(0);
    let mut l = Line { out, len: 0 };
    l.put(PREFIX);
    l.num(s.entry, entry_w);
    l.put(b" ");
    let at = l.len;
    name(list, i, &mut l);
    l.pad(at + name_w);
    l.put(b" ");
    match s.result {
        RAN => {
            l.put(b"ran ");
            l.num(s.fps, 0);
            l.put(b" fps");
        }
        FAILED => {
            l.put(b"failed, ");
            l.put(glmark2_why_name(s.why).as_bytes());
            let r = reason(s.error);
            if s.why != GLMARK2_WHY_NO_ERROR && !r.is_empty() && l.room() > 2 {
                l.put(b": ");
                let room = l.room();
                l.fit(if r.len() <= room { r } else { last_clause(r) }, room);
            }
        }
        UNSUPPORTED => l.put(b"unsupported"),
        NO_END => l.put(b"no end (the run stopped in it)"),
        other => {
            l.put(b"result ");
            l.num(other, 0);
            l.put(b" (unknown)");
        }
    }
    l.len
}

#[cfg(test)]
mod tests {
    use super::super::record::{GLMARK2_WHY_FRAGMENT_SHADER, GLMARK2_WHY_LINK};
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

    fn scene(info: &'static str, entry: u32, result: u32) -> Scene<'static> {
        Scene { info: info.as_bytes(), error: b"", entry, result, fps: 0, why: GLMARK2_WHY_NO_ERROR }
    }

    fn lines(list: &[Scene]) -> Vec<String> {
        (0..list.len())
            .map(|i| {
                let mut out = [0u8; 256];
                let n = line(list, i, &mut out);
                String::from_utf8(out[..n].to_vec()).unwrap()
            })
            .collect()
    }

    /// A run of the default list with every kind of result, as the sample
    /// prints it at the end.
    fn mixed_run() -> Vec<Scene<'static>> {
        DEFAULT_LIST
            .iter()
            .enumerate()
            .map(|(i, info)| {
                let mut s = scene(info, i as u32, RAN);
                s.fps = 119 + (i as u32 % 3);
                match i {
                    2..=4 => (s.result, s.why, s.error) = (FAILED, GLMARK2_WHY_LINK, LINK_ERROR.as_bytes()),
                    7 => {
                        (s.result, s.why) = (FAILED, GLMARK2_WHY_FRAGMENT_SHADER);
                        s.error = b"Error: Failed to add fragment shader from file None:\nError:   0:12: 'pow' : \
                            no matching overloaded function found";
                    }
                    17 | 19 => s.result = UNSUPPORTED,
                    22 => s.result = NO_END,
                    _ => {}
                }
                s
            })
            .collect()
    }

    #[test]
    fn the_default_list_reads_one_line_per_entry() {
        let l = lines(&mixed_run());
        // printed for the record of this module's output (cargo test -- --nocapture)
        for x in &l {
            println!("{x}");
        }
        assert_eq!(l.len(), 33);
        assert_eq!(l[0], "glmark2-list:  0 build:use-vbo=false                            ran 119 fps");
        assert_eq!(
            l[2],
            "glmark2-list:  2 texture:texture-filter=nearest                 failed, link: a texture coordinate \
             other than a vec2 varying r.."
        );
        // the whole log does not fit: its last clause does
        assert_eq!(
            l[7],
            "glmark2-list:  7 shading:shading=phong                          failed, fragment shader: no matching \
             overloaded function found"
        );
        assert_eq!(l[17], "glmark2-list: 17 buffer:interleave=false:update-method=map      unsupported");
        assert_eq!(
            l[22],
            "glmark2-list: 22 terrain                                        no end (the run stopped in it)"
        );
        for x in &l {
            assert!(x.len() <= LINE_MAX && x.bytes().all(|c| (0x20..0x7f).contains(&c)), "{x}");
        }
    }

    #[test]
    fn names_keep_the_options_that_tell_the_entries_of_a_scene_apart() {
        let list: Vec<_> = DEFAULT_LIST.iter().enumerate().map(|(i, info)| scene(info, i as u32, RAN)).collect();
        let names: Vec<String> = (0..list.len())
            .map(|i| {
                let mut out = [0u8; LINE_MAX];
                let mut l = Line { out: &mut out, len: 0 };
                name(&list, i, &mut l);
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
        assert_eq!(lines(&twice)[1], "glmark2-list: 1 build ran 0 fps");
        // a run cut short knows the entries so far only
        assert_eq!(lines(&list[..1])[0], "glmark2-list: 0 build ran 0 fps");
    }

    #[test]
    fn every_line_fits_whatever_the_list_holds() {
        let long = "[a-scene-name-longer-than-any-line-of-the-record-screen-can-show-in-one-row-of-text] \
                    <an-option=with-a-value-that-is-also-much-longer-than-a-name-may-be:second=1>";
        let error = format!("Error: Failed to link program created from files a and b:  {}\u{1}\n", "x".repeat(300));
        let error: &'static str = Box::leak(error.into_boxed_str());
        let mut list = vec![scene(long, 4_000_000_000, FAILED), scene("[build] <use-vbo=false>", 1, RAN)];
        list[0].why = GLMARK2_WHY_LINK;
        list[0].error = error.as_bytes();
        list[1].fps = u32::MAX;
        for result in [RAN, FAILED, UNSUPPORTED, NO_END, 7] {
            list[0].result = result;
            for x in lines(&list) {
                assert!(x.len() <= LINE_MAX && x.bytes().all(|c| (0x20..0x7f).contains(&c)), "{x}");
            }
        }
        list[0].result = FAILED;
        let l = lines(&list);
        assert!(l[0]
            .starts_with("glmark2-list: 4000000000 a-scene-name-longer-than-any-line-of-the-recor.. failed, link: x"));
        assert!(l[0].ends_with("x.."), "{}", l[0]);
        assert_eq!(l[1].len(), 14 + 10 + 1 + NAME_MAX + 1 + "ran 4294967295 fps".len());
        list[0].result = 7;
        assert!(lines(&list)[0].ends_with(" result 7 (unknown)"));
        // a short buffer is filled, not overrun
        let mut out = [0u8; 20];
        assert_eq!(line(&list, 1, &mut out), 20);
        assert!(out.starts_with(PREFIX));
        assert_eq!(line(&list, 2, &mut out), 0, "no such entry");
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
        // a failure without an error line says so, and no reason
        let mut s = scene("[shadow] <default>", 23, FAILED);
        s.error = b"Error: ignored without a why";
        assert_eq!(lines(&[s])[0], "glmark2-list: 23 shadow failed, no error logged");
    }
}
