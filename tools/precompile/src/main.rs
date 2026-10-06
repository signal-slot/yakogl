// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-precompile`: compile tools/shaders/precompiled.list with YakoGL's
//! own compiler, on the host, into a table an image can carry.
//!
//! ```text
//! yakogl-precompile [--list LIST] [--refusals FILE] [--refused-steps STEPS]
//!                   [--app NAME]... [-o OUT] [--verify TABLE] [--pin COUNTS]
//! ```
//!
//! It drives the product back end (`SHC_ONLINE`, src/shaderc/online) through
//! its own entry points in the order the GL core calls them ([`drive`]): each
//! listed program's two sources compiled, linked, and the variants a draw
//! asks for asked of the linked fragment shader; each source no program
//! links compiled alone, a fragment shader's variants asked of its plain
//! record. Then it writes every record the compiler's cache held, as Rust
//! ([`table`]), to OUT (default `build/precompiled/<gpu>[-es2].rs`).
//!
//! * A source the compiler refuses must be a listed refusal: FILE, default
//!   tests/fixtures/online_refused_<gpu>[_es2].txt, which
//!   tests/online_corpus.rs pins message for message. A listed one is
//!   reported and left out with the pairs that name it (`--skip-failures`
//!   by name); a refusal not listed, or a listed source that compiles now,
//!   fails the run.
//! * A link or a variant the compiler refuses (a program the GL refuses to
//!   link, a blend state whose draws it refuses) must be in STEPS, default
//!   tests/fixtures/precompile_refused_<gpu>[_es2].txt, one `<job>: <step>:
//!   <message>` line each in job order; the whole list's run must refuse
//!   exactly those (`PRECOMPILE_REFUSED_WRITE=1` writes what it saw), an
//!   app's run a subset of them.
//! * `--app NAME`: only that application's pairs (repeatable), and no source
//!   compiled alone: one application's sublist.
//! * `--verify TABLE`: make the table again in memory, then make every row
//!   of it again ALONE -- a fresh cache holding only what that one row needs
//!   (its source; for a link's record the pair, linked; for a variant that
//!   variant) -- and require TABLE, the regeneration and the table of fresh
//!   rows to be the same text. A row is then what the compiler makes for it,
//!   not an artefact of what else was in the cache when it was made.
//! * `--pin COUNTS`: require the row and code-byte counts to be COUNTS' line
//!   for this GPU, ES level and app set (`PRECOMPILE_PIN_WRITE=1` writes the
//!   line instead).
//!
//! The internal shaders (`internal/`: fbload, clear; GLSL 450 the back ends
//! carry themselves) are out of scope and skipped, by count, in the report.

#[cfg(not(any(feature = "gpu-powervr", feature = "gpu-vivante")))]
compile_error!("yakogl-precompile: enable exactly one GPU feature, gpu-powervr or gpu-vivante");
#[cfg(all(feature = "gpu-powervr", feature = "gpu-vivante"))]
compile_error!("yakogl-precompile: enable exactly one GPU feature, not both gpu-powervr and gpu-vivante");

mod drive;
mod list;
// The leaf printers of the shader ABI, shared with yakogl-tools' emit.rs.
#[path = "../../yakogl-tools/src/shc_print.rs"]
mod shc_print;
mod table;

use list::{List, Role};
use std::collections::{BTreeMap, HashMap, HashSet};
use table::{Origin, Row, Source, Step};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// The GPU this build compiles for, as the toolchain mark and the refusal
/// fixtures name it.
const GPU: &str = if cfg!(feature = "gpu-vivante") { "gc7000" } else { "powervr" };
/// 1 in a build that reports OpenGL ES 2.0.
const ES2: u8 = if cfg!(feature = "report-es2") { 1 } else { 0 };

/// The repository root: this crate is tools/precompile.
fn repo() -> String {
    let r = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    std::fs::canonicalize(r).map_or_else(|_| r.to_string(), |p| p.to_string_lossy().into_owned())
}

struct Args {
    list: String,
    refusals: String,
    refused_steps: String,
    apps: Vec<String>,
    output: String,
    verify: Option<String>,
    pin: Option<String>,
}

const USAGE: &str = "usage: yakogl-precompile [--list LIST] [--refusals FILE] [--refused-steps STEPS]
                         [--app NAME]... [-o OUT] [--verify TABLE] [--pin COUNTS]";

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let es2 = if ES2 == 1 { "_es2" } else { "" };
    let mut a = Args {
        list: format!("{}/tools/shaders/precompiled.list", repo()),
        refusals: format!("{}/tests/fixtures/online_refused_{GPU}{es2}.txt", repo()),
        refused_steps: format!("{}/tests/fixtures/precompile_refused_{GPU}{es2}.txt", repo()),
        apps: Vec::new(),
        output: format!("{}/build/precompiled/{GPU}{}.rs", repo(), if ES2 == 1 { "-es2" } else { "" }),
        verify: None,
        pin: None,
    };
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--list" => a.list = value()?,
            "--refusals" => a.refusals = value()?,
            "--refused-steps" => a.refused_steps = value()?,
            "--app" => a.apps.push(value()?),
            "-o" | "--output" => a.output = value()?,
            "--verify" => a.verify = Some(value()?),
            "--pin" => a.pin = Some(value()?),
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'\n{USAGE}")),
        }
    }
    Ok(a)
}

/// `path: message` per line, as tests/online_corpus.rs writes them.
fn read_refusals(path: &str) -> Result<HashMap<String, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut out = HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (p, m) = line.split_once(": ").ok_or_else(|| format!("{path}: not `path: message`: {line}"))?;
        out.insert(p.to_string(), m.to_string());
    }
    Ok(out)
}

/// The toolchain mark: the crate version (the workspace's one version, which
/// the library shares), the commit (`+dirty` when the tree is not that
/// commit), and the build's GPU and ES level.
fn toolchain() -> String {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(repo())
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = match (git(&["rev-parse", "HEAD"]), git(&["status", "--porcelain"])) {
        (Some(sha), Some(st)) => format!("{sha}{}", if st.is_empty() { "" } else { "+dirty" }),
        _ => "unknown".into(),
    };
    format!("yakogl={} commit={commit} gpu={GPU} es2={ES2}", env!("CARGO_PKG_VERSION"))
}

/// One unit of work: a program, or a source no selected program links.
#[derive(Clone, Debug)]
enum Job {
    Pair { vs: String, fs: String },
    Alone { src: String },
}

/// The list resolved for this GPU and app set.
struct Plan {
    list: List,
    /// Every source compiled, relative to tools/shaders, in order: the
    /// table's sources are these, less the ones no row uses.
    sources: Vec<Source>,
    /// A source text -> its first index in `sources` (a record names its
    /// source by text).
    by_text: HashMap<String, usize>,
    jobs: Vec<Job>,
    /// The listed refusals among the sources, with their messages.
    refused: Vec<(String, String)>,
    pairs_dropped: usize,
    internal: usize,
}

impl Plan {
    fn new(a: &Args) -> Result<Plan, String> {
        let list = List::read(&a.list)?;
        let stray = list.undeclared_dirs()?;
        if !stray.is_empty() {
            return Err(format!(
                "{}: directories under tools/shaders/ with no `dir` line: {}",
                a.list,
                stray.join(", ")
            ));
        }
        for app in &a.apps {
            if !list.apps().contains(&app.as_str()) {
                return Err(format!("--app {app}: the list has no such app ({})", list.apps().join(", ")));
            }
        }
        let refusals = read_refusals(&a.refusals)?;
        let internal = |rel: &str| list.dir(rel).is_some_and(|d| d.roles.contains(&Role::Internal));
        /* the GC7000's sources are its directories' (offarm/ is the PowerVR's) */
        let this_gpu = |rel: &str| {
            !internal(rel)
                && (cfg!(not(feature = "gpu-vivante"))
                    || list.dir(rel).is_some_and(|d| d.roles.contains(&Role::Gc7000)))
        };
        /* the programs: the selected apps', each once, on this GPU's sources */
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut pairs_dropped = 0;
        for p in list.pairs.iter().filter(|p| a.apps.is_empty() || a.apps.contains(&p.app)) {
            let pr = (p.vs.clone(), p.fs.clone());
            if pairs.contains(&pr) {
                continue;
            }
            if this_gpu(&p.vs) && this_gpu(&p.fs) {
                pairs.push(pr);
            } else {
                pairs_dropped += 1;
            }
        }
        /* the sources: the table's, then the pairs', each once */
        let base: Vec<String> = match (a.apps.is_empty(), cfg!(feature = "gpu-vivante")) {
            (false, _) => Vec::new(),
            (true, false) => list.srcs.clone(),
            (true, true) => list.sources_with(Role::Gc7000)?,
        };
        /* internal/ is out of this tool's scope for now: its sources are GLSL 450
         * the back ends carry themselves (src/pvr/blobs.rs, clear_shader.rs),
         * and no list line asks for them; counted, so the report says so */
        let internal_n = list.sources_with(Role::Internal)?.len();
        let mut names: Vec<String> = Vec::new();
        for s in base.iter().chain(pairs.iter().flat_map(|(v, f)| [v, f])) {
            if internal(s) {
                return Err(format!(
                    "{s}: an internal source; the list asks for it, and this tool does not compile those"
                ));
            }
            if !names.contains(s) {
                names.push(s.clone());
            }
        }
        let mut sources = Vec::with_capacity(names.len());
        let mut by_text = HashMap::new();
        for (i, s) in names.iter().enumerate() {
            let text = std::fs::read_to_string(list.abs(s)).map_err(|e| format!("{s}: {e}"))?;
            by_text.entry(text.clone()).or_insert(i);
            sources.push(Source { name: s.clone(), text });
        }
        /* the listed refusals are left out, with every pair naming one */
        let refused: Vec<(String, String)> =
            names.iter().filter_map(|s| refusals.get(s).map(|m| (s.clone(), m.clone()))).collect();
        let gone: HashSet<&String> = refused.iter().map(|(s, _)| s).collect();
        let before = pairs.len();
        pairs.retain(|(v, f)| !gone.contains(v) && !gone.contains(f));
        pairs_dropped += before - pairs.len();
        let mut jobs: Vec<Job> = pairs.iter().map(|(vs, fs)| Job::Pair { vs: vs.clone(), fs: fs.clone() }).collect();
        let paired: HashSet<&String> = pairs.iter().flat_map(|(v, f)| [v, f]).collect();
        if a.apps.is_empty() {
            jobs.extend(
                names
                    .iter()
                    .filter(|s| !paired.contains(s) && !gone.contains(s))
                    .map(|s| Job::Alone { src: s.clone() }),
            );
        }
        let pl = Plan { list, sources, by_text, jobs, refused, pairs_dropped, internal: internal_n };
        pl.check_refusals()?;
        Ok(pl)
    }

    /// A listed refusal is still refused, and with its message: a list that
    /// has rotted says so rather than excusing a source that compiles now.
    fn check_refusals(&self) -> Result<(), String> {
        for (s, want) in &self.refused {
            let mut h = drive::Holders::default();
            let got = drive::compile(&mut h, drive::stage_of(s), self.text(s));
            h.release_all()?;
            match got {
                Err(m) if &m == want => {}
                Err(m) => return Err(format!("{s}: refused, but not as listed\n    listed: {want}\n    now:    {m}")),
                Ok(_) => return Err(format!("{s}: listed as refused, and it compiles (re-derive the fixture)")),
            }
        }
        Ok(())
    }

    fn text(&self, rel: &str) -> &str {
        self.sources.iter().find(|s| s.name == rel).map_or("", |s| &s.text)
    }

    /// The variants a draw asks of fragment shader `rel` (its program's
    /// linked copy `fs`): on the PowerVR the list's specs for it, on the
    /// GC7000 the opposite winding of a shader that reads gl_FrontFacing.
    fn keys(&self, rel: &str, fs: &ShcShader) -> Result<Vec<u32>, String> {
        if cfg!(feature = "gpu-vivante") {
            return Ok(if fs.reads_front_facing != 0 {
                vec![yakogl::shaderc::SHC_VIV_KEY_FRONT_FLIPPED]
            } else {
                vec![]
            });
        }
        self.list.specs_for(rel).iter().map(|s| list::blend_key(s).map_err(|e| format!("{rel}: {s}: {e:?}"))).collect()
    }
}

/// What a run made: the rows by identity, and every link by its two
/// shaders' identities (the vertex shader's key, the fragment shader's key
/// and blend key) to the fragment shader's key after it.
#[derive(Default)]
struct Made {
    rows: BTreeMap<drive::Id, Row>,
    links: BTreeMap<([u8; 32], [u8; 32], u32), [u8; 32]>,
}

impl Made {
    /// Record a link; one recorded before must have come out the same.
    fn link(&mut self, l: drive::Link) -> Result<(), String> {
        let k = (l.vs, l.fs, l.fs_blend_key);
        match self.links.insert(k, l.linked_fs) {
            Some(had) if had != l.linked_fs => {
                Err(format!("the link of {} and {} came out two ways", drive::hex(&l.vs[..8]), drive::hex(&l.fs[..8])))
            }
            _ => Ok(()),
        }
    }
}

/// Run `job` -- or, with `only`, the part of it that makes one step's
/// records -- capturing every record into `made` (the first step to make a
/// record is its origin).
fn run(
    plan: &Plan,
    job: &Job,
    only: Option<(u32, Step)>,
    made: &mut Made,
    refused: &mut Vec<String>,
) -> Result<(), String> {
    let mut h = drive::Holders::default();
    let r = run_held(plan, job, only, made, refused, &mut h);
    let released = h.release_all();
    r.and(released)
}

fn run_held(
    plan: &Plan,
    job: &Job,
    only: Option<(u32, Step)>,
    made: &mut Made,
    refused: &mut Vec<String>,
    h: &mut drive::Holders,
) -> Result<(), String> {
    let origin = |step| match job {
        Job::Pair { vs, fs } => (Origin::Pair { vs: vs.clone(), fs: fs.clone() }, step),
        Job::Alone { src } => (Origin::Alone { src: src.clone() }, step),
    };
    let wants = |step: Step| only.is_none_or(|(_, s)| s == step);
    let compile = |h: &mut drive::Holders, rel: &str| {
        drive::compile(h, drive::stage_of(rel), plan.text(rel)).map_err(|m| format!("{rel}: refused, not listed: {m}"))
    };
    match job {
        Job::Alone { src } => {
            let sh = compile(h, src)?;
            take(plan, made, origin(Step::Compile))?;
            if drive::stage_of(src) == SHC_STAGE_FRAGMENT {
                for k in plan.keys(src, &sh)?.into_iter().filter(|&k| wants(Step::Variant(k))) {
                    match drive::variant_unlinked(h, &sh, k) {
                        Ok(()) => take(plan, made, origin(Step::Variant(k)))?,
                        Err(m) => refused.push(format!("{src}: variant 0x{k:x}: {m}")),
                    }
                }
            }
        }
        Job::Pair { vs, fs } => {
            /* a compile's record alone: just that source */
            if let Some((stage, Step::Compile)) = only {
                compile(h, if stage == SHC_STAGE_FRAGMENT { fs } else { vs })?;
                return take(plan, made, origin(Step::Compile));
            }
            let vsh = compile(h, vs)?;
            let fsh = compile(h, fs)?;
            take(plan, made, origin(Step::Compile))?;
            /* a program the GL refuses to link: no draw asks it for a variant */
            let (rvs, rfs) = match drive::link(h, plan.text(vs), &vsh, &fsh) {
                Ok((rvs, rfs, link)) => {
                    made.link(link)?;
                    (rvs, rfs)
                }
                Err(m) => {
                    refused.push(format!("pair {vs} {fs}: link: {m}"));
                    return Ok(());
                }
            };
            take(plan, made, origin(Step::Link))?;
            /* a variant the compiler refuses is a blend state whose draws the
             * GL refuses (gl::draw::fs_variant_for); the others still come */
            for k in plan.keys(fs, &rfs)?.into_iter().filter(|&k| wants(Step::Variant(k))) {
                match drive::variant(h, &rvs, &rfs, k) {
                    Ok(link) => {
                        made.link(link)?;
                        take(plan, made, origin(Step::Variant(k)))?
                    }
                    Err(m) => refused.push(format!("pair {vs} {fs}: variant 0x{k:x}: {m}")),
                }
            }
        }
    }
    Ok(())
}

/// Copy every record the cache holds that `made` lacks. One it has must be
/// the same row: a cache key covering less than the compile read would show
/// here as two rows under one identity.
fn take(plan: &Plan, made: &mut Made, origin: (Origin, Step)) -> Result<(), String> {
    let (_, overflowed, _, _) = yakogl::shaderc::online::backend::stats();
    if overflowed != 0 {
        return Err(format!("the compiler's cache overflowed ({overflowed} records not indexed)"));
    }
    let mut err = None;
    let mut new: Vec<(drive::Id, yakogl::shaderc::ShcShader)> = Vec::new();
    yakogl::shaderc::online::backend::each_record(|r| {
        if err.is_some() {
            return;
        }
        let id = (r.key, r.stage, r.blend_key);
        /* every record of a run is compiled under the image's default `if`
         * lowering (drive::lend sets it), which the key does not chain */
        if r.if_lowering != yakogl::shaderc::IfLowering::Default {
            err = Some(format!("record {} was compiled under {:?}", drive::hex(&r.key), r.if_lowering));
            return;
        }
        let Some(&src) = std::str::from_utf8(r.source).ok().and_then(|t| plan.by_text.get(t)) else {
            err = Some(format!("record {} has a source this run did not compile", drive::hex(&r.key)));
            return;
        };
        let mut row = Row::of(r, src, origin.clone());
        match made.rows.get(&id) {
            None => {
                new.push((id, r.shader));
                made.rows.insert(id, row);
            }
            Some(had) => {
                /* the sensitivity is asked once per identity (below); `--verify`
                 * asks it again of the row made alone */
                row.sensitivity = had.sensitivity;
                let (x, y) = (table::fingerprint(had, &plan.sources), table::fingerprint(&row, &plan.sources));
                if x.is_err() || x != y {
                    err = Some(format!("record {} came out of two jobs different", row.ident()));
                }
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    /* each new row's sampler-state sensitivity, asked of the live record as
     * a program's first draw asks it (after the walk: the question runs the
     * front end, which must not run inside the cache's iteration) */
    for (id, sh) in new {
        if let Some(row) = made.rows.get_mut(&id) {
            row.sensitivity = drive::sensitivity(&sh);
        }
    }
    Ok(())
}

/// The rows in table order, re-indexed onto the sources they use.
fn finish(plan: &Plan, made: Made) -> (Vec<Row>, Vec<Source>) {
    let mut used: Vec<usize> = made.rows.values().map(|r| r.source).collect();
    used.sort_unstable();
    used.dedup();
    let mut rows: Vec<Row> = made.rows.into_values().collect();
    for r in &mut rows {
        r.source = used.binary_search(&r.source).unwrap_or(usize::MAX);
    }
    rows.sort_by_key(|r| (r.source, r.stage, r.blend_key, r.key));
    let sources = used
        .into_iter()
        .map(|i| Source { name: plan.sources[i].name.clone(), text: plan.sources[i].text.clone() })
        .collect();
    (rows, sources)
}

/// Every row of `rows` made again alone (see `--verify`), in the same order.
fn fresh(plan: &Plan, rows: &[Row], sources: &[Source]) -> Result<Vec<Row>, String> {
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let (origin, step) = &r.origin;
        let job = match origin {
            Origin::Pair { vs, fs } => Job::Pair { vs: vs.clone(), fs: fs.clone() },
            Origin::Alone { src } => Job::Alone { src: src.clone() },
        };
        let mut made = Made::default();
        let mut refused = Vec::new();
        run(plan, &job, Some((r.stage, *step)), &mut made, &mut refused)?;
        if !refused.is_empty() {
            return Err(format!("{}: made alone, a step was refused: {}", r.ident(), refused.join("; ")));
        }
        let Some(mut row) = made.rows.remove(&r.id()) else {
            return Err(format!("{}: made alone, its step made no such record", r.ident()));
        };
        /* the table's numbering of its source, and the origin it is listed with */
        let name = &plan.sources[row.source].name;
        row.source = sources.iter().position(|s| &s.name == name).unwrap_or(usize::MAX);
        row.origin = r.origin.clone();
        out.push(row);
    }
    Ok(out)
}

/// The links and variants refused must be STEPS' (see the module comment).
fn check_refused_steps(a: &Args, refused: &[String]) -> Result<(), String> {
    let path = &a.refused_steps;
    let got: String = refused.iter().map(|l| format!("{l}\n")).collect();
    if a.apps.is_empty() && std::env::var_os("PRECOMPILE_REFUSED_WRITE").is_some() {
        std::fs::write(path, &got).map_err(|e| format!("{path}: {e}"))?;
        println!("yakogl-precompile: wrote {path} ({} refused steps)", refused.len());
        return Ok(());
    }
    let want = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let listed: HashSet<&str> = want.lines().collect();
    if let Some(l) = refused.iter().find(|l| !listed.contains(l.as_str())) {
        return Err(format!("refused, and not as {path} lists it:\n    {l}"));
    }
    if a.apps.is_empty() && got != want {
        let seen: HashSet<&str> = refused.iter().map(String::as_str).collect();
        let stale: Vec<&str> = want.lines().filter(|l| !seen.contains(l)).take(3).collect();
        return Err(format!(
            "{path} lists steps this run did not refuse (or in another order): {stale:?} (PRECOMPILE_REFUSED_WRITE=1 \
             re-derives it)"
        ));
    }
    Ok(())
}

/// COUNTS' key for this run.
fn pin_key(apps: &[String]) -> String {
    format!("{GPU} es2={ES2} app={}", if apps.is_empty() { "all".to_string() } else { apps.join("+") })
}

/// `key: value` must be COUNTS' line for `key` (or is written there).
fn check_pin(path: &str, key: &str, value: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let line = format!("{key}: {value}");
    if std::env::var_os("PRECOMPILE_PIN_WRITE").is_some() {
        let (head, mut body): (Vec<&str>, Vec<&str>) = text
            .lines()
            .filter(|l| !l.starts_with(&format!("{key}: ")))
            .partition(|l| l.starts_with('#') || l.is_empty());
        body.push(&line);
        body.sort_unstable();
        let all: Vec<&str> = head.into_iter().chain(body).collect();
        std::fs::write(path, all.join("\n") + "\n").map_err(|e| format!("{path}: {e}"))?;
        println!("yakogl-precompile: pinned in {path}: {line}");
        return Ok(());
    }
    match text.lines().find(|l| l.starts_with(&format!("{key}: "))) {
        Some(l) if l == line => {
            println!("yakogl-precompile: the counts are {path}'s pin: {line}");
            Ok(())
        }
        Some(l) => Err(format!(
            "{path}: the counts moved\n    pinned: {l}\n    now:    {line}\n  (PRECOMPILE_PIN_WRITE=1 re-pins; say why in the commit)"
        )),
        None => Err(format!("{path}: no line for `{key}` (PRECOMPILE_PIN_WRITE=1 writes it)")),
    }
}

/// One run's table: its rows, its sources and its text.
struct Generated {
    plan: Plan,
    rows: Vec<Row>,
    links: Vec<yakogl::shaderc::ShcPrecompiledLink>,
    sources: Vec<Source>,
    about: Vec<String>,
    text: String,
}

/// Make the table `a` asks for (the compiler's regions lent already).
fn generate(a: &Args, mark: &str) -> Result<Generated, String> {
    let plan = Plan::new(a)?;
    let mut made = Made::default();
    let mut refused = Vec::new();
    for job in &plan.jobs {
        run(&plan, job, None, &mut made, &mut refused)?;
    }
    check_refused_steps(a, &refused)?;
    let links: Vec<yakogl::shaderc::ShcPrecompiledLink> = made
        .links
        .iter()
        .map(|(&(vs, fs, fs_blend_key), &linked_fs)| yakogl::shaderc::ShcPrecompiledLink {
            vs,
            fs,
            fs_blend_key,
            linked_fs,
        })
        .collect();
    let (rows, sources) = finish(&plan, made);
    for l in &links {
        for (key, blend) in [(l.vs, 0), (l.fs, l.fs_blend_key), (l.linked_fs, l.fs_blend_key)] {
            if !rows.iter().any(|r| r.key == key && r.blend_key == blend) {
                return Err(format!("a link names {} (blend 0x{blend:x}), which is no row", drive::hex(&key[..8])));
            }
        }
    }
    let about = vec![
        format!(
            "Apps: {}.",
            if a.apps.is_empty() { "every one, and every listed source".to_string() } else { a.apps.join(", ") }
        ),
        format!(
            "Left out: {} source(s) the compiler refuses as {} lists them, {} pair(s) naming one or off this GPU, \
             internal/ ({} sources: GLSL 450 the back ends carry themselves).",
            plan.refused.len(),
            a.refusals.rsplit('/').next().unwrap_or(&a.refusals),
            plan.pairs_dropped,
            plan.internal
        ),
        format!(
            "{} link(s) and variant(s) refused as {} lists them.",
            refused.len(),
            a.refused_steps.rsplit('/').next().unwrap_or("")
        ),
    ];
    let text = table::emit(
        &rows,
        &sources,
        &links,
        &table::Header { toolchain: mark, gpu: GPU, es2: ES2 == 1, about: &about },
    )?;
    Ok(Generated { plan, rows, links, sources, about, text })
}

/// `--verify`: every row of `g` made again alone, and the table they make
/// is `g`'s text and `had`.
fn verify(g: &Generated, mark: &str, had: &str, path: &str) -> Result<(), String> {
    let again = fresh(&g.plan, &g.rows, &g.sources)?;
    for (r, f) in g.rows.iter().zip(&again) {
        if table::fingerprint(r, &g.sources)? != table::fingerprint(f, &g.sources)? {
            return Err(format!("{}: the row is not what the compiler makes for it alone", r.ident()));
        }
    }
    if table::emit(
        &again,
        &g.sources,
        &g.links,
        &table::Header { toolchain: mark, gpu: GPU, es2: ES2 == 1, about: &g.about },
    )? != g.text
    {
        return Err("the table of fresh rows is not the regenerated table".into());
    }
    if had != g.text {
        let at = had.lines().zip(g.text.lines()).position(|(x, y)| x != y);
        let at = at.unwrap_or(had.lines().count().min(g.text.lines().count()));
        return Err(format!(
            "{path} is not the table this list and compiler make; first difference at line {}\n    file: {}\n    made: {}",
            at + 1,
            had.lines().nth(at).unwrap_or("<end>"),
            g.text.lines().nth(at).unwrap_or("<end>")
        ));
    }
    Ok(())
}

fn main_r(argv: &[String]) -> Result<(), String> {
    let a = parse_args(argv)?;
    drive::lend()?;
    let t0 = std::time::Instant::now();
    let mark = toolchain();
    let g = generate(&a, &mark)?;
    let (rows, plan) = (&g.rows, &g.plan);
    let apps = if a.apps.is_empty() { "all".to_string() } else { a.apps.join("+") };
    let vs = rows.iter().filter(|r| r.stage != SHC_STAGE_FRAGMENT).count();
    let bytes: usize = rows.iter().map(Row::code_bytes).sum();
    println!(
        "yakogl-precompile: {GPU} es2={ES2} app={apps}: {} rows (vs {vs}, fs {}) of {} sources, {bytes} code bytes, \
         table {} bytes; {} jobs, {:.1} s; refused as listed {}, pairs left out {}, internal/ skipped ({} sources)",
        rows.len(),
        rows.len() - vs,
        g.sources.len(),
        g.text.len(),
        plan.jobs.len(),
        t0.elapsed().as_secs_f64(),
        plan.refused.len(),
        plan.pairs_dropped,
        plan.internal
    );
    if let Some(path) = &a.verify {
        let t1 = std::time::Instant::now();
        let had = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        verify(&g, &mark, &had, path)?;
        println!(
            "yakogl-precompile: verify: {path} is the regeneration, and each of its {} rows is what the compiler \
             makes for that row alone ({:.1} s)",
            rows.len(),
            t1.elapsed().as_secs_f64()
        );
    } else {
        if let Some(d) = std::path::Path::new(&a.output).parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        /* an image build runs this every time: a table that did not change
         * is not touched, so the library that includes it is not rebuilt */
        if std::fs::read_to_string(&a.output).ok().as_deref() == Some(g.text.as_str()) {
            println!("yakogl-precompile: {} is unchanged", a.output);
        } else {
            std::fs::write(&a.output, &g.text).map_err(|e| format!("{}: {e}", a.output))?;
            println!("yakogl-precompile: wrote {}", a.output);
        }
    }
    if let Some(path) = &a.pin {
        check_pin(path, &pin_key(&a.apps), &format!("rows={} code_bytes={bytes}", rows.len()))?;
    }
    Ok(())
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = main_r(&argv) {
        eprintln!("yakogl-precompile: error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, Once};

    /// The compiler is single-threaded by contract and the library is built
    /// as an image takes it (no `host-test`, so no lock of its own): the
    /// tests that compile take this one. The regions are lent once.
    fn compiler() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        static LENT: Once = Once::new();
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        LENT.call_once(|| drive::lend().expect("the regions are lent"));
        g
    }

    fn args(apps: &[&str]) -> Args {
        let mut a = parse_args(&[]).expect("the defaults");
        a.apps = apps.iter().map(|s| s.to_string()).collect();
        a
    }

    /// A tree of its own under the repository's build/, with a list and two
    /// empty refusal files.
    fn tree(name: &str, files: &[(&str, &str)], list: &str) -> Args {
        let root = format!("{}/build/test-tmp/precompile-{name}", repo());
        let _ = std::fs::remove_dir_all(&root);
        for (f, text) in files.iter().chain([("none.txt", ""), ("l.list", list)].iter()) {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(format!("{root}/{f}"), text).unwrap();
        }
        let mut a = args(&[]);
        a.list = format!("{root}/l.list");
        a.refusals = format!("{root}/none.txt");
        a.refused_steps = format!("{root}/none.txt");
        a
    }

    /// `backend::each_record`, which every row is read through, visits a live
    /// record with what a row copies out of it: the key a plain compile's
    /// look-up uses (the source's SHA-256, nothing chained), the source
    /// without its NUL, no facts, and the shader as handed out; a job's
    /// release empties the cache.
    #[test]
    fn each_record_visits_the_live_records_as_compiled() {
        let _c = compiler();
        let text = "precision mediump float; void main(){ gl_FragColor = vec4(0.625); } // each_record\n";
        let mut h = drive::Holders::default();
        let sh = drive::compile(&mut h, SHC_STAGE_FRAGMENT, text).expect("compiles");
        let mut seen = Vec::new();
        yakogl::shaderc::online::backend::each_record(|r| {
            seen.push((
                r.key,
                r.stage,
                r.blend_key,
                r.source == text.as_bytes(),
                r.w_constant,
                r.lod_key,
                r.shader.priv_,
            ));
        });
        let key = yakogl::shaderc::sha256::sha256(text.as_bytes());
        let none = yakogl::shaderc::LodKey::NONE;
        assert_eq!(seen, [(key, SHC_STAGE_FRAGMENT, 0, true, false, none, sh.priv_)]);
        h.release_all().expect("nothing left held");
        let mut after = 0;
        yakogl::shaderc::online::backend::each_record(|_| after += 1);
        assert_eq!(after, 0, "released, it is gone from the cache");
    }

    /// A row's key is the key an image's look-up computes for the same
    /// source under the image's default `if` lowering (the generator's
    /// `drive::lend` sets it): compiled at run time with the default, the
    /// source's record has the row's key; compiled with the switch on
    /// branching (PVR_BACKEND_OPT_IF_LOWERING), it has another, which no
    /// row of the table has -- a miss, compiled at run time, not a wrong hit.
    #[test]
    fn a_rows_key_is_the_runtime_key_under_the_default_if_lowering() {
        use yakogl::shaderc::{set_if_lowering, IfLowering};
        let _c = compiler();
        let g = generate(&args(&["femtovg"]), "test").expect("femtovg's table");
        let row = g
            .rows
            .iter()
            .find(|r| r.stage == SHC_STAGE_FRAGMENT && r.blend_key == 0 && r.origin.1 == Step::Compile)
            .expect("a plain fragment row");
        assert_eq!(row.if_lowering, IfLowering::Default.as_u32());
        let text = &g.sources[row.source].text;
        let key_under = |l: IfLowering| {
            set_if_lowering(l.as_u32());
            let mut h = drive::Holders::default();
            let sh = drive::compile(&mut h, SHC_STAGE_FRAGMENT, text).expect("compiles");
            let mut key = None;
            yakogl::shaderc::online::backend::each_record(|r| {
                if core::ptr::eq(r.shader.priv_, sh.priv_) {
                    key = Some(r.key);
                }
            });
            h.release_all().expect("released");
            set_if_lowering(IfLowering::Default.as_u32());
            key.expect("its record")
        };
        assert_eq!(key_under(IfLowering::Default), row.key, "the image's default look-up hits the row");
        let branched = key_under(IfLowering::Branch);
        assert_ne!(branched, row.key);
        assert!(g.rows.iter().all(|r| r.key != branched), "a branching image's look-up misses every row");
    }

    /// The list every generator reads parses, and every directory under
    /// tools/shaders/ with a shader in it has a `dir` line.
    #[test]
    fn the_list_reads_and_accounts_for_every_directory() {
        let l = List::read(&args(&[]).list).expect("tools/shaders/precompiled.list reads");
        assert_eq!(l.undeclared_dirs().unwrap(), Vec::<String>::new());
        assert!(l.srcs.len() > 500, "the PowerVR inputs, qt/ and qt3d/ expanded: {}", l.srcs.len());
        assert_eq!(l.apps(), ["femtovg", "gl_test", "gl_test_es100"]);
        assert_eq!(l.specs_for("qt3d/a.frag").len(), 5, "the first matching `for` wins");
        assert_eq!(l.specs_for("femtovg/fill_color.frag"), &l.blends[..]);
    }

    /// One app's table, made twice, is the same text, and each of its rows is
    /// what the compiler makes for that row alone. On the PowerVR the link's
    /// own records are among them (femtovg's glyph shaders sample with
    /// gradients and main.vert writes a constant W, so the link compiles them
    /// again without), and a variant the compiler refuses is one STEPS lists.
    #[test]
    fn an_apps_table_is_deterministic_and_each_row_is_a_fresh_compile() {
        let _c = compiler();
        let a = args(&["femtovg"]);
        let g = generate(&a, "test").expect("femtovg's table");
        let again = generate(&a, "test").expect("again");
        assert_eq!(g.text, again.text, "the same list and compiler make the same table");
        assert!(g.rows.len() > 35, "{} rows", g.rows.len());
        verify(&g, "test", &g.text.clone(), "the regeneration").expect("each row is a fresh compile");
        if cfg!(feature = "gpu-powervr") {
            assert!(g.rows.iter().any(|r| r.w_constant && r.origin.1 == Step::Link), "a link's record");
            assert!(g.rows.iter().any(|r| r.w_constant && matches!(r.origin.1, Step::Variant(_))), "its variants");
        }
        let mut doctored = g.text.clone();
        let at = doctored.find("_USC: [u32; ").expect("a code array");
        let word = at + doctored[at..].find("0x").expect("a word");
        doctored.replace_range(word..word + 10, "0xdeadbeef");
        let e = verify(&g, "test", &doctored, "doctored").expect_err("a word altered");
        assert!(e.contains("first difference"), "{e}");
    }

    /// A refusal the fixtures do not list fails the run, for a source and
    /// for a variant alike; with the source listed, it is left out with its
    /// pair and the run goes on.
    #[test]
    fn a_refusal_must_be_listed() {
        let _c = compiler();
        let good_vs = "attribute vec4 p;\nvoid main(){ gl_Position = p; }\n";
        let bad_fs = "precision mediump float;\nvoid main(){ gl_FragColor = undeclared; }\n";
        let a = tree(
            "refusal",
            &[("p.vert", good_vs), ("bad.frag", bad_fs)],
            "dir . gc7000\nsrc p.vert\nsrc bad.frag\napp x\npair p.vert bad.frag\n",
        );
        let e = generate(&a, "test").err().expect("an unlisted refusal");
        assert!(
            e.contains("bad.frag: refused, not listed: ") && e.contains("'undeclared' : undeclared identifier"),
            "{e}"
        );
        let mut listed = a;
        let fixture = format!("{}.refused", listed.list);
        let msg = e.split("not listed: ").nth(1).unwrap().to_string();
        std::fs::write(&fixture, format!("bad.frag: {msg}\n")).unwrap();
        listed.refusals = fixture;
        let g = generate(&listed, "test").expect("listed, it is left out");
        assert_eq!((g.rows.len(), g.plan.pairs_dropped, g.plan.refused.len()), (1, 1, 1), "p.vert alone");
        if cfg!(feature = "gpu-powervr") {
            let mut a = args(&["femtovg"]);
            a.refused_steps = listed.refusals.replace(".refused", ".none");
            std::fs::write(&a.refused_steps, "").unwrap();
            let e = generate(&a, "test").err().expect("femtovg's refused constant-colour variant, unlisted");
            assert!(e.contains("refused, and not as") && e.contains("variant 0xb9ee7"), "{e}");
        }
    }

    /// A vertex shader the compile leaves to the link (the GC7000 places 17
    /// attributes only with the program's locations) is compiled again with
    /// the map the link makes, and that record is a row with its map.
    #[test]
    #[cfg(feature = "gpu-vivante")]
    fn a_link_time_layout_is_a_row_with_its_attribute_map() {
        let _c = compiler();
        let mut vs = String::from("varying mediump vec4 v;\n");
        for k in 0..17 {
            vs += &format!("attribute mediump float a_{k};\n");
        }
        vs += "void main(){\n\tmediump vec4 c = vec4(0.0);\n";
        for k in 0..16 {
            vs += &format!("\tc += vec4(a_{k});\n");
        }
        vs += "\tv = c;\n\tgl_Position = c;\n}\n";
        let fs = "varying mediump vec4 v;\nvoid main(){ gl_FragColor = v; }\n";
        let a = tree("layout", &[("a.vert", &vs), ("c.frag", fs)], "dir . gc7000\napp x\npair a.vert c.frag\n");
        let g = generate(&a, "test").expect("the pair");
        let laid: Vec<&Row> = g.rows.iter().filter(|r| r.map.is_some()).collect();
        assert_eq!(laid.len(), 1, "one record compiled with the map");
        let m = laid[0].map.as_ref().unwrap();
        assert_eq!(m.count, 17);
        let locs: Vec<u8> = m.b[..17].iter().map(|b| b.location).collect();
        let want: Vec<u8> = (0..16).chain([yakogl::shaderc::SHC_ATTRIB_NO_LOCATION]).collect();
        assert_eq!(locs, want, "the lowest free locations in order; the unread one none");
        assert!(g.text.contains(&format!("Some(&{}_ATTRIB_MAP)", laid[0].ident())));
        assert!(
            g.rows.iter().any(|r| r.shader.layout_at_link == 1 && r.shader.num_declared == 17),
            "the deferred compile"
        );
        verify(&g, "test", &g.text.clone(), "the regeneration").expect("each row is a fresh compile");
    }
}
