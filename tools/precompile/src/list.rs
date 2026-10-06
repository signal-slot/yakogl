// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `tools/shaders/precompiled.list`, the shader sources every table is built
//! from, and the blend-variant spec syntax its `blend` and `for` lines use.
//!
//! The list's syntax is documented at its head; this is its one reader in
//! Rust. Two tools compile it in: `yakogl-precompile` (this crate) and
//! `yakogl-tools glsl2usc --sources` (`#[path]`), so the PowerVR Mesa table
//! and our compiler's table cannot read the list two ways. The spec parser
//! below is glsl2usc's, moved here for the same reason; glsl2usc formats its
//! errors as it always has. (gen_shader_table.py reads the `dir` lines only,
//! with its own three lines of Python.)

use std::collections::HashSet;
use yakogl::shaderc as abi;

/* ---- the blend-variant spec: FACTORS[:MASK][@RT] -------------------------- */

/// Blend factor codes (Mesa pipe_blendfactor == enum shc_blend_factor).
pub const BLEND_FACTORS: &[(&str, u32)] = &[
    ("one", abi::SHC_BF_ONE),
    ("src_color", abi::SHC_BF_SRC_COLOR),
    ("src_alpha", abi::SHC_BF_SRC_ALPHA),
    ("dst_alpha", abi::SHC_BF_DST_ALPHA),
    ("dst_color", abi::SHC_BF_DST_COLOR),
    ("src_alpha_saturate", abi::SHC_BF_SRC_ALPHA_SATURATE),
    ("const_color", abi::SHC_BF_CONST_COLOR),
    ("const_alpha", abi::SHC_BF_CONST_ALPHA),
    ("zero", abi::SHC_BF_ZERO),
    ("inv_src_color", abi::SHC_BF_INV_SRC_COLOR),
    ("inv_src_alpha", abi::SHC_BF_INV_SRC_ALPHA),
    ("inv_dst_alpha", abi::SHC_BF_INV_DST_ALPHA),
    ("inv_dst_color", abi::SHC_BF_INV_DST_COLOR),
    ("inv_const_color", abi::SHC_BF_INV_CONST_COLOR),
    ("inv_const_alpha", abi::SHC_BF_INV_CONST_ALPHA),
];
pub const BLEND_FUNCS: &[(&str, u32)] = &[
    ("add", abi::SHC_BLEND_ADD),
    ("sub", abi::SHC_BLEND_SUBTRACT),
    ("rsub", abi::SHC_BLEND_REVERSE_SUBTRACT),
    ("min", abi::SHC_BLEND_MIN),
    ("max", abi::SHC_BLEND_MAX),
];
pub const COLORMASK_BITS: &[(char, u32)] = &[('r', 1), ('g', 2), ('b', 4), ('a', 8)];
/// Render-target classes a fragment shader can be compiled for (the colour
/// output packed for the target's texel format, pco_shc PCO_SHC_OUT).
pub const RT_CLASSES: &[(&str, u32)] = &[
    ("", abi::SHC_RT_RGBA8),
    ("rgba8", abi::SHC_RT_RGBA8),
    ("f16x4", abi::SHC_RT_F16X4),
    ("f16", abi::SHC_RT_F16),
    ("f32", abi::SHC_RT_F32),
];

pub fn lookup(table: &[(&str, u32)], k: &str) -> Option<u32> {
    table.iter().find(|(n, _)| *n == k).map(|(_, v)| *v)
}

/// Why a spec does not parse. Each reader words it its own way (glsl2usc's
/// messages are pinned by its tests).
#[derive(Debug, PartialEq, Eq)]
pub enum SpecError {
    /// `@RT` names no render target class.
    RtClass { rt: String },
    /// `:MASK` has a character other than r g b a.
    Mask { mask: String },
    /// Fewer than two factors.
    TooFewFactors,
    /// A factor or equation name that is not one.
    Unknown(String),
    /// The spec is ONE/ZERO with every channel written into RGBA8: the plain
    /// shader, not a variant of it.
    PlainReplace,
}

/// The render target class names, for a message.
pub fn rt_class_names() -> String {
    RT_CLASSES.iter().map(|(n, _)| *n).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ")
}

/// `s` split at the first `sep` (Python's str.partition, as glsl2usc used).
fn partition(s: &str, sep: char) -> (&str, &str) {
    s.split_once(sep).unwrap_or((s, ""))
}

/// 'BLENDSPEC[@rt]' -> (blendspec, rt).
pub fn split_rt(spec: &str) -> Result<(&str, &str), SpecError> {
    let (blend, rt) = partition(spec, '@');
    if lookup(RT_CLASSES, rt).is_none() {
        return Err(SpecError::RtClass { rt: rt.to_string() });
    }
    Ok((blend, rt))
}

/// 'FACTORS[:MASK][@rt]' -> (factors, mask bits).
pub fn split_variant_spec(spec: &str) -> Result<(String, u32), SpecError> {
    let (spec, _) = split_rt(spec)?;
    let (factors, mask) = partition(spec, ':');
    let factors = if factors == "replace" { "one,zero" } else { factors };
    let mut mask_bits = 0xf;
    if !mask.is_empty() {
        mask_bits = 0;
        for ch in mask.chars() {
            match COLORMASK_BITS.iter().find(|(c, _)| *c == ch) {
                Some((_, b)) => mask_bits |= b,
                None => return Err(SpecError::Mask { mask: mask.to_string() }),
            }
        }
    }
    Ok((factors.to_string(), mask_bits))
}

/// Variant spec -> the blend key (SHC_BLEND_KEY | MASK_OFF | RT).
pub fn blend_key(spec: &str) -> Result<u32, SpecError> {
    let (factors, mask_bits) = split_variant_spec(spec)?;
    let t: Vec<&str> = factors.split(',').collect();
    if t.len() < 2 {
        return Err(SpecError::TooFewFactors);
    }
    let bf = |k: &str| lookup(BLEND_FACTORS, k).ok_or_else(|| SpecError::Unknown(k.to_string()));
    let bq = |k: &str| lookup(BLEND_FUNCS, k).ok_or_else(|| SpecError::Unknown(k.to_string()));
    let srgb = bf(t[0])?;
    let drgb = bf(t[1])?;
    let sa = if t.len() > 2 { bf(t[2])? } else { srgb };
    let da = if t.len() > 3 { bf(t[3])? } else { drgb };
    let eq = if t.len() > 4 { bq(t[4])? } else { abi::SHC_BLEND_ADD };
    let eqa = if t.len() > 5 { bq(t[5])? } else { eq };
    let mut key = abi::shc_blend_key(srgb, drgb, sa, da, eq, eqa);
    key |= abi::shc_blend_key_mask_off(!mask_bits & 0xf);
    key |= abi::shc_blend_key_rt(lookup(RT_CLASSES, split_rt(spec)?.1).unwrap_or(0));
    if key == abi::SHC_BLEND_KEY_REPLACE {
        return Err(SpecError::PlainReplace);
    }
    Ok(key)
}

/// A spec error in the list's words.
fn spec_message(spec: &str, e: &SpecError) -> String {
    match e {
        SpecError::RtClass { rt } => {
            format!("'{spec}': render target class '{rt}' is not one of {}", rt_class_names())
        }
        SpecError::Mask { mask } => format!("'{spec}': mask '{mask}': channels are r g b a"),
        SpecError::TooFewFactors => format!("'{spec}': needs at least srgb,drgb"),
        SpecError::Unknown(n) => format!("'{spec}': unknown blend factor/equation '{n}'"),
        SpecError::PlainReplace => format!("'{spec}' is the plain replace the base shader already is"),
    }
}

/* ---- globs ---------------------------------------------------------------- */

/// `name` matches `pat`, where `*` is any run of characters (`/` included,
/// as Python's fnmatch, which glsl2usc's `--for` uses) and `?` any one. The
/// list allows no other metacharacter ([`List::parse`] refuses `[`), so this
/// is fnmatch on the list's own subset.
pub fn fnmatch(name: &str, pat: &str) -> bool {
    let (n, p): (Vec<char>, Vec<char>) = (name.chars().collect(), pat.chars().collect());
    /* the classic two-pointer match with one backtrack point per `*` */
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while i < n.len() {
        if j < p.len() && p[j] == '*' {
            star = Some((j, i));
            j += 1;
        } else if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if let Some((sj, si)) = star {
            j = sj + 1;
            i = si + 1;
            star = Some((sj, si + 1));
        } else {
            return false;
        }
    }
    p[j..].iter().all(|&c| c == '*')
}

/* ---- the list ------------------------------------------------------------- */

/// What a directory under tools/shaders/ is for (`dir DIR ROLE...`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The GC7000 table takes every shader in it.
    Gc7000,
    /// The back ends' own shaders: no table holds them.
    Internal,
}

#[derive(Clone, Debug)]
pub struct Dir {
    /// Relative to tools/shaders/, "" for tools/shaders itself (`.` in the list).
    pub path: String,
    pub roles: Vec<Role>,
}

/// One `pair` line: a program `app` links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub app: String,
    pub vs: String,
    pub fs: String,
}

/// The GPU an `internal` line is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    Powervr,
    Gc7000,
}

/// What an `internal` program is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InternalFrom {
    /// A source under an `internal` directory, and for a fragment shader the
    /// variant (`SPEC`'s blend key) the back end binds, 0 for the plain one.
    Source { path: String, blend_key: u32, spec: Option<String> },
    /// `attach:N`: the PowerVR's clear-attach program of N dwords
    /// (`powervr::assemble::build_clear_attach`).
    Attach(u32),
}

/// One `internal` line: a program a back end binds itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Internal {
    pub gpu: Gpu,
    pub name: String,
    pub from: InternalFrom,
}

/// The list, read and checked against the tree it names.
#[derive(Clone, Debug)]
pub struct List {
    /// tools/shaders/ as the caller named it (the list's own directory).
    pub root: String,
    pub dirs: Vec<Dir>,
    /// The `src` lines, globs expanded, relative to `root`, in list order.
    pub srcs: Vec<String>,
    /// The `blend` lines.
    pub blends: Vec<String>,
    /// The `for` lines: (GLOB, SPECs), relative to `root`, in list order.
    pub rules: Vec<(String, Vec<String>)>,
    pub skip_failures: bool,
    pub pairs: Vec<Pair>,
    /// The `internal` lines, in list order.
    pub internal: Vec<Internal>,
}

/// The directory a relative path lies in ("" for the root).
pub fn dir_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(d, _)| d)
}

impl List {
    /// Read `path`; its directory is tools/shaders/, which every path in it
    /// is relative to.
    pub fn read(path: &str) -> Result<List, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let root = match path.rsplit_once('/') {
            Some((d, _)) if !d.is_empty() => d.to_string(),
            Some(_) => "/".to_string(),
            None => ".".to_string(),
        };
        List::parse(&text, &root).map_err(|e| format!("{path}:{e}"))
    }

    /// The list in `text`, its paths relative to `root`. Errors are
    /// `LINE: message`.
    pub fn parse(text: &str, root: &str) -> Result<List, String> {
        let mut l = List {
            root: root.to_string(),
            dirs: Vec::new(),
            srcs: Vec::new(),
            blends: Vec::new(),
            rules: Vec::new(),
            skip_failures: false,
            pairs: Vec::new(),
            internal: Vec::new(),
        };
        let mut app: Option<String> = None;
        let mut seen_src = HashSet::new();
        for (n, line) in text.lines().enumerate() {
            let n = n + 1;
            let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
            let Some(&kw) = f.first() else {
                continue;
            };
            let args = &f[1..];
            let want = |k: usize, shape: &str| {
                if args.len() == k {
                    Ok(())
                } else {
                    Err(format!("{n}: `{shape}` takes {k} field(s), this line has {}", args.len()))
                }
            };
            match kw {
                "dir" => {
                    if args.len() < 2 {
                        return Err(format!("{n}: `dir DIR ROLE...` needs a directory and at least one role"));
                    }
                    let path = if args[0] == "." { String::new() } else { args[0].trim_end_matches('/').to_string() };
                    let mut roles = Vec::new();
                    for r in &args[1..] {
                        match *r {
                            "gc7000" => roles.push(Role::Gc7000),
                            "internal" => roles.push(Role::Internal),
                            "-" if args.len() == 2 => {}
                            other => {
                                return Err(format!("{n}: dir role '{other}' is not gc7000, internal or a lone -"))
                            }
                        }
                    }
                    if l.dirs.iter().any(|d| d.path == path) {
                        return Err(format!("{n}: dir '{}' is listed twice", args[0]));
                    }
                    if !std::path::Path::new(&l.abs(&path)).is_dir() {
                        return Err(format!("{n}: dir '{}' is not a directory under {root}", args[0]));
                    }
                    l.dirs.push(Dir { path, roles });
                }
                "src" => {
                    want(1, "src PATH")?;
                    let found = l.expand(args[0]).map_err(|e| format!("{n}: {e}"))?;
                    for rel in found {
                        l.listed_dir(&rel).map_err(|e| format!("{n}: {e}"))?;
                        if !seen_src.insert(rel.clone()) {
                            return Err(format!("{n}: src '{rel}' is listed twice"));
                        }
                        l.srcs.push(rel);
                    }
                }
                "blend" => {
                    want(1, "blend SPEC")?;
                    blend_key(args[0]).map_err(|e| format!("{n}: blend {}", spec_message(args[0], &e)))?;
                    l.blends.push(args[0].to_string());
                }
                "for" => {
                    want(2, "for GLOB SPEC[;SPEC...]")?;
                    no_brackets(args[0]).map_err(|e| format!("{n}: {e}"))?;
                    let specs: Vec<String> = args[1].split(';').filter(|x| !x.is_empty()).map(String::from).collect();
                    if specs.is_empty() {
                        return Err(format!("{n}: `for {}` names no spec", args[0]));
                    }
                    for s in &specs {
                        blend_key(s).map_err(|e| format!("{n}: for {}", spec_message(s, &e)))?;
                    }
                    l.rules.push((args[0].to_string(), specs));
                }
                "skip-failures" => {
                    want(0, "skip-failures")?;
                    l.skip_failures = true;
                }
                "app" => {
                    want(1, "app NAME")?;
                    if l.pairs.iter().any(|p| p.app == args[0]) {
                        return Err(format!("{n}: app '{}' has a second block", args[0]));
                    }
                    app = Some(args[0].to_string());
                }
                "pair" => {
                    want(2, "pair VS FS")?;
                    let Some(app) = &app else {
                        return Err(format!("{n}: a pair before any `app` line"));
                    };
                    let (vs, fs) = (args[0], args[1]);
                    if !vs.ends_with(".vert") || !fs.ends_with(".frag") {
                        return Err(format!("{n}: pair {vs} {fs}: a .vert and then a .frag"));
                    }
                    for p in [vs, fs] {
                        if !std::path::Path::new(&l.abs(p)).is_file() {
                            return Err(format!("{n}: {p}: no such file under {root}"));
                        }
                        l.listed_dir(p).map_err(|e| format!("{n}: {e}"))?;
                    }
                    let pair = Pair { app: app.clone(), vs: vs.to_string(), fs: fs.to_string() };
                    if l.pairs.contains(&pair) {
                        return Err(format!("{n}: pair {vs} {fs} is listed twice in app {app}"));
                    }
                    l.pairs.push(pair);
                }
                "internal" => {
                    if !(3..=4).contains(&args.len()) {
                        return Err(format!("{n}: `internal GPU NAME SOURCE [SPEC]` takes 3 or 4 fields"));
                    }
                    let gpu = match args[0] {
                        "powervr" => Gpu::Powervr,
                        "gc7000" => Gpu::Gc7000,
                        other => return Err(format!("{n}: internal GPU '{other}' is not powervr or gc7000")),
                    };
                    let name = args[1];
                    if name.is_empty()
                        || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    {
                        return Err(format!("{n}: internal NAME '{name}' is not lower-case letters, digits and _"));
                    }
                    if l.internal.iter().any(|i| i.gpu == gpu && i.name == name) {
                        return Err(format!("{n}: internal {} {name} is listed twice", args[0]));
                    }
                    let from = if let Some(k) = args[2].strip_prefix("attach:") {
                        let k: u32 = k.parse().map_err(|_| format!("{n}: attach:{k}: not a dword count"))?;
                        if gpu != Gpu::Powervr || args.len() != 3 || !(1..=3).contains(&k) {
                            return Err(format!(
                                "{n}: attach:N is the PowerVR's clear-attach program of 1 to 3 dwords, with no SPEC"
                            ));
                        }
                        InternalFrom::Attach(k)
                    } else {
                        let path = args[2];
                        if !std::path::Path::new(&l.abs(path)).is_file() {
                            return Err(format!("{n}: {path}: no such file under {root}"));
                        }
                        if !l.dir(path).is_some_and(|d| d.roles.contains(&Role::Internal)) {
                            return Err(format!("{n}: {path}: not in an `internal` directory"));
                        }
                        let spec = args.get(3).map(|s| s.to_string());
                        if spec.is_some() && !path.ends_with(".frag") {
                            return Err(format!("{n}: {path}: a variant SPEC is a fragment shader's"));
                        }
                        let blend_key = match &spec {
                            Some(s) => blend_key(s).map_err(|e| format!("{n}: internal {}", spec_message(s, &e)))?,
                            None => 0,
                        };
                        InternalFrom::Source { path: path.to_string(), blend_key, spec }
                    };
                    l.internal.push(Internal { gpu, name: name.to_string(), from });
                }
                other => return Err(format!("{n}: unknown line kind '{other}'")),
            }
        }
        if l.dirs.is_empty() {
            return Err("0: no `dir` lines: the list names no directory".into());
        }
        Ok(l)
    }

    /// `rel` under the root.
    pub fn abs(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{rel}", self.root)
        }
    }

    /// The `dir` line `rel` lies in.
    pub fn dir(&self, rel: &str) -> Option<&Dir> {
        let d = dir_of(rel);
        self.dirs.iter().find(|x| x.path == d)
    }

    fn listed_dir(&self, rel: &str) -> Result<(), String> {
        match self.dir(rel) {
            Some(_) => Ok(()),
            None => Err(format!("{rel}: its directory '{}' has no `dir` line", dir_of(rel))),
        }
    }

    /// A `src` path: itself, or the files its last component's glob matches
    /// in its directory, in byte order (dot files excepted, as a shell glob).
    fn expand(&self, pat: &str) -> Result<Vec<String>, String> {
        no_brackets(pat)?;
        let (d, name) = pat.rsplit_once('/').unwrap_or(("", pat));
        if d.contains(['*', '?']) {
            return Err(format!("src {pat}: a glob only in the file name"));
        }
        if !name.contains(['*', '?']) {
            if !std::path::Path::new(&self.abs(pat)).is_file() {
                return Err(format!("src {pat}: no such file under {}", self.root));
            }
            return Ok(vec![pat.to_string()]);
        }
        let mut out = Vec::new();
        let rd = std::fs::read_dir(self.abs(d)).map_err(|e| format!("src {pat}: {e}"))?;
        for e in rd {
            let e = e.map_err(|e| format!("src {pat}: {e}"))?;
            let f = e.file_name().to_string_lossy().into_owned();
            if !f.starts_with('.') && e.path().is_file() && fnmatch(&f, name) {
                out.push(if d.is_empty() { f } else { format!("{d}/{f}") });
            }
        }
        out.sort();
        Ok(out)
    }

    /// The variant specs of fragment shader `rel`: the first `for` whose glob
    /// matches it, else every `blend`.
    pub fn specs_for(&self, rel: &str) -> &[String] {
        self.rules.iter().find(|(g, _)| fnmatch(rel, g)).map_or(&self.blends, |(_, s)| s)
    }

    /// The directories under the root that hold a .vert or .frag and have no
    /// `dir` line, sorted: what every reader refuses (the TABLE_DIRS guard).
    pub fn undeclared_dirs(&self) -> Result<Vec<String>, String> {
        let mut found = Vec::new();
        walk(&self.root, "", &mut found)?;
        let mut out: Vec<String> =
            found.iter().map(|f| dir_of(f).to_string()).filter(|d| !self.dirs.iter().any(|x| &x.path == d)).collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Every .vert and .frag of the directories with `role`, relative to the
    /// root, in byte order.
    pub fn sources_with(&self, role: Role) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for d in self.dirs.iter().filter(|d| d.roles.contains(&role)) {
            let rd = std::fs::read_dir(self.abs(&d.path)).map_err(|e| format!("{}: {e}", self.abs(&d.path)))?;
            for e in rd {
                let e = e.map_err(|e| format!("{}: {e}", d.path))?;
                let f = e.file_name().to_string_lossy().into_owned();
                if e.path().is_file() && (f.ends_with(".vert") || f.ends_with(".frag")) {
                    out.push(if d.path.is_empty() { f } else { format!("{}/{f}", d.path) });
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// The `app` names, in list order.
    pub fn apps(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for p in &self.pairs {
            if !out.contains(&p.app.as_str()) {
                out.push(&p.app);
            }
        }
        out
    }
}

fn no_brackets(pat: &str) -> Result<(), String> {
    if pat.contains(['[', ']']) {
        return Err(format!("{pat}: a list glob has `*` and `?` only"));
    }
    Ok(())
}

/// Every .vert and .frag under `root/rel`, relative to `root`.
fn walk(root: &str, rel: &str, out: &mut Vec<String>) -> Result<(), String> {
    let here = if rel.is_empty() { root.to_string() } else { format!("{root}/{rel}") };
    for e in std::fs::read_dir(&here).map_err(|e| format!("{here}: {e}"))? {
        let e = e.map_err(|e| format!("{here}: {e}"))?;
        let f = e.file_name().to_string_lossy().into_owned();
        let r = if rel.is_empty() { f.clone() } else { format!("{rel}/{f}") };
        if e.path().is_dir() {
            walk(root, &r, out)?;
        } else if f.ends_with(".vert") || f.ends_with(".frag") {
            out.push(r);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_are_fnmatch_on_the_lists_subset() {
        assert!(fnmatch("qt3d/a.frag", "qt3d/*.frag"));
        assert!(fnmatch("qt3d/sub/a.frag", "qt3d/*"), "`*` crosses `/`, as Python's fnmatch");
        assert!(!fnmatch("qt/a.frag", "qt3d/*"));
        assert!(fnmatch("es300/x.frag", "es300/?.frag"));
        assert!(!fnmatch("es300/xy.frag", "es300/?.frag"));
        assert!(fnmatch("a", "**a*"));
        assert!(!fnmatch("abc", "a*d"));
        assert!(fnmatch("", "*"));
        assert!(fnmatch("*a", "*"), "a `*` in the name is a character like any other");
    }

    #[test]
    fn blend_keys() {
        assert_eq!(blend_key("src_alpha,inv_src_alpha"), Ok(0x0009_8e63));
        assert_eq!(blend_key("src_alpha,inv_src_alpha:rgb"), Ok(0x2009_8e63));
        assert_eq!(blend_key("replace@f16"), Ok(abi::SHC_BLEND_KEY_REPLACE | 0x8000_0000));
        assert_eq!(blend_key("replace"), Err(SpecError::PlainReplace));
        assert_eq!(blend_key("one,bogus"), Err(SpecError::Unknown("bogus".into())));
        assert_eq!(blend_key("one,one@f64"), Err(SpecError::RtClass { rt: "f64".into() }));
        assert_eq!(blend_key("one,one:x@f64"), Err(SpecError::RtClass { rt: "f64".into() }), "the class first");
        assert_eq!(blend_key("one,one:x"), Err(SpecError::Mask { mask: "x".into() }));
        assert_eq!(blend_key("one"), Err(SpecError::TooFewFactors));
    }

    /// A small tree, written for the test under the repository's build/
    /// (per crate: two compile this file in, and their tests may run at once).
    fn tree(name: &str, files: &[&str]) -> String {
        let root =
            format!("{}/../../build/test-tmp/{}-list-{name}", env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"));
        let _ = std::fs::remove_dir_all(&root);
        for f in files {
            let p = format!("{root}/{f}");
            std::fs::create_dir_all(dir_of(&p)).unwrap();
            std::fs::write(&p, "void main(){}\n").unwrap();
        }
        root
    }

    #[test]
    fn a_list_reads_and_its_globs_expand_in_byte_order() {
        let root = tree("ok", &["a.vert", "b.frag", "q/Z.frag", "q/a.frag", "q/.x.frag", "int/c.frag"]);
        let l = List::parse(
            "dir . gc7000\ndir q gc7000 # c\ndir int internal\nsrc a.vert\nsrc b.frag\nsrc q/*.frag\n\
             blend one,one\nfor q/* replace@f16;one,inv_src_alpha\nskip-failures\napp x\npair a.vert q/a.frag\n",
            &root,
        )
        .unwrap();
        assert_eq!(l.srcs, ["a.vert", "b.frag", "q/Z.frag", "q/a.frag"], "byte order, no dot files");
        assert_eq!(l.specs_for("b.frag"), ["one,one"]);
        assert_eq!(l.specs_for("q/a.frag"), ["replace@f16", "one,inv_src_alpha"]);
        assert!(l.skip_failures);
        assert_eq!(l.pairs, [Pair { app: "x".into(), vs: "a.vert".into(), fs: "q/a.frag".into() }]);
        assert_eq!(
            l.sources_with(Role::Gc7000).unwrap(),
            ["a.vert", "b.frag", "q/.x.frag", "q/Z.frag", "q/a.frag"],
            "every shader of the directory, as gen_shader_table.py's os.walk takes them"
        );
        assert_eq!(l.undeclared_dirs().unwrap(), Vec::<String>::new());
    }

    /// `internal` lines: a source of an internal directory and its variant,
    /// or a clear-attach program; each refusal by line.
    #[test]
    fn internal_lines() {
        let root = tree("int", &["a.vert", "int/q.vert", "int/c.frag"]);
        let head = "dir . gc7000\ndir int internal\n";
        let l = List::parse(
            &format!(
                "{head}internal powervr quad int/q.vert\ninternal powervr m1 int/c.frag replace:r@f16x4\n\
                 internal powervr a2 attach:2\ninternal gc7000 quad int/q.vert\n"
            ),
            &root,
        )
        .unwrap();
        let src = |path: &str, blend_key: u32, spec: Option<&str>| InternalFrom::Source {
            path: path.into(),
            blend_key,
            spec: spec.map(String::from),
        };
        assert_eq!(
            l.internal,
            [
                Internal { gpu: Gpu::Powervr, name: "quad".into(), from: src("int/q.vert", 0, None) },
                Internal {
                    gpu: Gpu::Powervr,
                    name: "m1".into(),
                    from: src("int/c.frag", blend_key("replace:r@f16x4").unwrap(), Some("replace:r@f16x4")),
                },
                Internal { gpu: Gpu::Powervr, name: "a2".into(), from: InternalFrom::Attach(2) },
                Internal { gpu: Gpu::Gc7000, name: "quad".into(), from: src("int/q.vert", 0, None) },
            ]
        );
        let refused = |text: &str, want: &str| {
            let e = List::parse(&format!("{head}{text}"), &root).expect_err(text);
            assert!(e.contains(want), "{text:?}: {e}");
        };
        refused("internal mali x int/q.vert\n", "3: internal GPU 'mali'");
        refused("internal powervr X int/q.vert\n", "NAME 'X'");
        refused(
            "internal powervr x int/q.vert\ninternal powervr x int/c.frag\n",
            "4: internal powervr x is listed twice",
        );
        refused("internal powervr x a.vert\n", "a.vert: not in an `internal` directory");
        refused("internal powervr x int/none.frag\n", "no such file");
        refused("internal powervr x int/q.vert replace:r\n", "a variant SPEC is a fragment shader's");
        refused("internal powervr x int/c.frag replace\n", "the plain replace");
        refused("internal gc7000 x attach:1\n", "the PowerVR's clear-attach program");
        refused("internal powervr x attach:4\n", "1 to 3 dwords");
        refused("internal powervr x\n", "takes 3 or 4 fields");
    }

    /// Each refusal, so that a reader that stopped checking one says so.
    #[test]
    fn a_list_that_is_wrong_is_refused_by_line() {
        let root = tree("bad", &["a.vert", "b.frag", "stray/w.vert"]);
        let refused = |text: &str, want: &str| {
            let e = List::parse(text, &root).expect_err(text);
            assert!(e.contains(want), "{text:?}: {e}");
        };
        refused("src a.vert\n", "1: a.vert: its directory '' has no `dir` line");
        refused("dir . gc7000\nsrc nope.vert\n", "2: src nope.vert: no such file");
        refused("dir . gc7000\nsrc a.vert\nsrc a.vert\n", "3: src 'a.vert' is listed twice");
        refused("dir . gc7000\nblend replace\n", "2: blend 'replace' is the plain replace");
        refused("dir . gc7000\nfor *.frag one,one;one,bogus\n", "unknown blend factor/equation 'bogus'");
        refused("dir . gc7000\nfor [ab].frag one,one\n", "`*` and `?` only");
        refused("dir . gc7000\npair a.vert b.frag\n", "a pair before any `app` line");
        refused("dir . gc7000\napp x\npair b.frag a.vert\n", "a .vert and then a .frag");
        refused("dir . mali\n", "dir role 'mali'");
        refused("dir . gc7000\ndir . -\n", "listed twice");
        refused("dir nowhere -\n", "not a directory");
        refused("dir . gc7000\nwhat ever\n", "unknown line kind 'what'");
        refused("# nothing\n", "no `dir` lines");
        let l = List::parse("dir . gc7000\n", &root).unwrap();
        assert_eq!(l.undeclared_dirs().unwrap(), ["stray"], "a directory nobody was told about");
    }
}
