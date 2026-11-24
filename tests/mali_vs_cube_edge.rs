// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Mesa 26.2.1's references for issue #7489 (item cube-ref, plan #406) parse
//! and agree with each other: a vertex-shader cube look-up with `GL_REPEAT`
//! and a `LINEAR` filter near a face edge (dEQP-GLES2
//! texture.vertex.cube.filtering.linear_mipmap_linear_{linear,nearest}_repeat
//! and .wrap.repeat_repeat, row 5 of docs/es2-cts-failures.md section M.1)
//! and textureCubeLod (shaders.texture_functions.vertex.texturecubelod, row
//! 11, and a fragment-stage counterpart).
//!
//! The references are `tools/mali/testdata/vs_cube_edge*` and `cube_lod*`,
//! written by `tools/mali/testdata/vs_cube_edge_ref.sh` on Mesa's drm-shim
//! (no board): per program a `.expect.json` (the programs' words and
//! disassembly, `pan_ref_diff.py extract --draw`) and per case and context a
//! `.desc` (the draw's sampler and texture descriptors as pandecode decodes
//! them, then their raw words from the memory dump, with the texture's
//! plane descriptors). The contexts are `es3` (what Mesa makes for a GLES 2
//! request: ES 3.x, every cube map seamless) and `es2`
//! (`MESA_GLES_VERSION_OVERRIDE=2.0`: no seamless filtering, ES 3.0
//! Appendix F.2).
//!
//! This test needs no GPU and no Mesa: it reads the committed files and
//! asserts that
//!
//! - every raw descriptor word set unpacks with our `genxml` to the fields
//!   pandecode decoded, and packs back to the same words (samplers,
//!   textures, AFBC planes);
//! - each case's sampler holds its GL state: in `es3` seamless with every
//!   wrap clamped to the edge (st/mesa's `st_convert_sampler` drops the
//!   wraps of a seamless cube), in `es2` not seamless with `GL_REPEAT` kept
//!   on R, T and S; the filters are the case's; the two contexts' samplers
//!   differ in those four fields only, and their textures not at all;
//! - the textures are the cases' cube maps (32x32, 6 levels, RGB; 256x256,
//!   9 levels, RGBA) with Levels x 6 planes;
//! - the programs sample with Mesa's cube sequence (CUBEFACE1, CUBEFACE2_V9,
//!   CUBE_SSEL, CUBE_TSEL, TEX_SINGLE .cube.explicit) in the stage the case
//!   names, and `vs_cube_edge` is `pan_tex_ref/vcube` (the same shader pair
//!   and, `trace` aside, the same `.expect.json`).
//!
//! ```text
//! cargo test --test mali_vs_cube_edge --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use std::collections::BTreeMap;
use yakogl::mali::genxml::v10::{
    AfbcPlane, DescriptorType, Func, LodAlgorithm, MipmapMode, PlaneType, Sampler, Texture, TextureDimension, WrapMode,
};

/* ---- the files ------------------------------------------------------------- */

fn testdata(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/mali/testdata").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// One `.desc` reference: its case, its context and the GL state the case
/// sets (`vs_cube_edge_draw.py`).
struct Case {
    file: &'static str,
    es2: bool,
    /// The resource tables the look-up's stage reads.
    tables: &'static [&'static str],
    mag_nearest: bool,
    mipmap: MipmapMode,
    size: u32,
    levels: u32,
}

const VS: &[&str] = &["Position", "Varying"];
const FS: &[&str] = &["Fragment"];

const CASES: &[Case] = &[
    Case {
        file: "vs_cube_edge.lml_linear_repeat.edge.es3.desc",
        es2: false,
        tables: VS,
        mag_nearest: false,
        mipmap: MipmapMode::TRILINEAR,
        size: 32,
        levels: 6,
    },
    Case {
        file: "vs_cube_edge.lml_linear_repeat.edge.es2.desc",
        es2: true,
        tables: VS,
        mag_nearest: false,
        mipmap: MipmapMode::TRILINEAR,
        size: 32,
        levels: 6,
    },
    Case {
        file: "vs_cube_edge.lml_nearest_repeat.edge.es3.desc",
        es2: false,
        tables: VS,
        mag_nearest: true,
        mipmap: MipmapMode::TRILINEAR,
        size: 32,
        levels: 6,
    },
    Case {
        file: "vs_cube_edge.lml_nearest_repeat.edge.es2.desc",
        es2: true,
        tables: VS,
        mag_nearest: true,
        mipmap: MipmapMode::TRILINEAR,
        size: 32,
        levels: 6,
    },
    Case {
        file: "cube_lod_vs.lmn_linear_repeat.es3.desc",
        es2: false,
        tables: VS,
        mag_nearest: false,
        mipmap: MipmapMode::NEAREST,
        size: 256,
        levels: 9,
    },
    Case {
        file: "cube_lod_vs.lmn_linear_repeat.es2.desc",
        es2: true,
        tables: VS,
        mag_nearest: false,
        mipmap: MipmapMode::NEAREST,
        size: 256,
        levels: 9,
    },
    Case {
        file: "cube_lod_fs.lmn_linear_repeat.es3.desc",
        es2: false,
        tables: FS,
        mag_nearest: false,
        mipmap: MipmapMode::NEAREST,
        size: 256,
        levels: 9,
    },
];

/* ---- reading a `.desc` ----------------------------------------------------- */

type Fields = BTreeMap<String, String>;

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The fields of the lines below `lines[at]` indented deeper than it, up to
/// the first nested block (a line ending in `:`).
fn fields_below(lines: &[&str], at: usize) -> Fields {
    let base = indent(lines[at]);
    let mut f = Fields::new();
    for l in lines[at + 1..].iter().take_while(|l| indent(l) > base) {
        if l.trim_end().ends_with(':') {
            break;
        }
        let (k, v) = l.trim().split_once(": ").unwrap_or_else(|| panic!("a decoded field: {l}"));
        assert!(f.insert(k.to_string(), v.to_string()).is_none(), "{k} twice");
    }
    f
}

/// The decoded descriptor `kind` ("Sampler" or "Texture") of entry `entry`
/// of resource table `table`, with the index of its header line.
fn decoded(lines: &[&str], table: &str, entry: u32, kind: &str) -> (usize, Fields) {
    let t = lines
        .iter()
        .position(|l| l.starts_with(&format!("{table} resource table @")))
        .unwrap_or_else(|| panic!("a {table} table"));
    let e = t + lines[t..]
        .iter()
        .position(|l| *l == format!("  Entry {entry}:"))
        .unwrap_or_else(|| panic!("{table} entry {entry}"));
    let d = e + lines[e..]
        .iter()
        .position(|l| l.trim_start().starts_with(&format!("{kind} @")))
        .unwrap_or_else(|| panic!("{table} entry {entry}: a {kind}"));
    // a texture's fields sit under its `Texture:` line
    let head = if kind == "Texture" { d + 1 } else { d };
    (head, fields_below(lines, head))
}

/// One `words` line of a `.desc`.
struct Raw {
    table: String,
    entry: u32,
    kind: String,
    va: u64,
    bytes: [u8; 32],
    /// A texture's plane descriptors.
    planes: Vec<[u8; 32]>,
}

fn bytes_of(words: &str) -> [u8; 32] {
    let w: Vec<u32> = words.split(' ').map(|w| u32::from_str_radix(w, 16).expect("a hex word")).collect();
    assert_eq!(w.len(), 8, "eight words: {words}");
    let mut b = [0u8; 32];
    for (k, w) in w.iter().enumerate() {
        b[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
    }
    b
}

fn raws(desc: &str) -> Vec<Raw> {
    let at = desc.lines().position(|l| l == "words").expect("a words section");
    let mut out: Vec<Raw> = Vec::new();
    for l in desc.lines().skip(at + 1) {
        if let Some(p) = l.strip_prefix("    plane ") {
            let (n, words) = p.split_once(": ").expect("plane <n>: words");
            let last = out.last_mut().expect("a texture before its planes");
            assert_eq!(n.parse::<usize>().unwrap(), last.planes.len(), "planes in order");
            last.planes.push(bytes_of(words));
        } else if l.starts_with("    planes: ") {
            continue;
        } else {
            let (head, words) = l.trim_start().split_once(": ").expect("<table> entry <n> <kind> @<va>: words");
            let h: Vec<&str> = head.split(' ').collect();
            assert_eq!((h.len(), h[1]), (5, "entry"), "{head}");
            out.push(Raw {
                table: h[0].to_string(),
                entry: h[2].parse().unwrap(),
                kind: h[3].to_string(),
                va: u64::from_str_radix(h[4].trim_start_matches('@'), 16).unwrap(),
                bytes: bytes_of(words),
                planes: Vec::new(),
            });
        }
    }
    out
}

fn take(f: &mut Fields, k: &str) -> String {
    f.remove(k).unwrap_or_else(|| panic!("no {k}"))
}

fn flag(f: &mut Fields, k: &str) -> bool {
    match take(f, k).as_str() {
        "true" => true,
        "false" => false,
        v => panic!("{k}: {v}"),
    }
}

fn hex(v: &str) -> u64 {
    u64::from_str_radix(v.split(' ').next().unwrap().trim_start_matches("0x"), 16).unwrap()
}

fn wrap_named(v: &str) -> WrapMode {
    match v {
        "Repeat" => WrapMode::REPEAT,
        "Clamp to Edge" => WrapMode::CLAMP_TO_EDGE,
        "Mirrored Repeat" => WrapMode::MIRRORED_REPEAT,
        _ => panic!("wrap mode {v}"),
    }
}

/// A sampler from pandecode's fields (every field read; an unknown value
/// panics).
fn sampler_of(mut f: Fields) -> Sampler {
    let f = &mut f;
    assert_eq!((take(f, "Type"), take(f, "Reduction mode")), ("Sampler".into(), "Average".into()));
    let s = Sampler {
        wrap_mode_r: wrap_named(&take(f, "Wrap Mode R")),
        wrap_mode_t: wrap_named(&take(f, "Wrap Mode T")),
        wrap_mode_s: wrap_named(&take(f, "Wrap Mode S")),
        round_to_nearest_even: flag(f, "Round to nearest even"),
        srgb_override: flag(f, "sRGB override"),
        seamless_cube_map: flag(f, "Seamless Cube Map"),
        clamp_integer_coordinates: flag(f, "Clamp integer coordinates"),
        normalized_coordinates: flag(f, "Normalized Coordinates"),
        clamp_integer_array_indices: flag(f, "Clamp integer array indices"),
        minify_nearest: flag(f, "Minify nearest"),
        magnify_nearest: flag(f, "Magnify nearest"),
        magnify_cutoff: flag(f, "Magnify cutoff"),
        mipmap_mode: match take(f, "Mipmap Mode").as_str() {
            "None" => MipmapMode::NONE,
            "Nearest" => MipmapMode::NEAREST,
            "Trilinear" => MipmapMode::TRILINEAR,
            v => panic!("mipmap mode {v}"),
        },
        minimum_lod: take(f, "Minimum LOD").parse().unwrap(),
        compare_function: match take(f, "Compare Function").as_str() {
            "Never" => Func::NEVER,
            v => panic!("compare function {v}"),
        },
        maximum_lod: take(f, "Maximum LOD").parse().unwrap(),
        lod_bias: take(f, "LOD bias").parse().unwrap(),
        maximum_anisotropy: take(f, "Maximum anisotropy").parse().unwrap(),
        lod_algorithm: match take(f, "LOD algorithm").as_str() {
            "Isotropic" => LodAlgorithm::ISOTROPIC,
            v => panic!("LOD algorithm {v}"),
        },
        border_color_r: hex(&take(f, "Border Color R")) as u32,
        border_color_g: hex(&take(f, "Border Color G")) as u32,
        border_color_b: hex(&take(f, "Border Color B")) as u32,
        border_color_a: hex(&take(f, "Border Color A")) as u32,
        ..Default::default()
    };
    assert!(f.is_empty(), "fields not read: {f:?}");
    s
}

fn num(f: &mut Fields, k: &str) -> u64 {
    let v = take(f, k);
    match v.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).unwrap(),
        None => v.split(' ').next().unwrap().parse().unwrap_or_else(|_| panic!("{k}: {v}")),
    }
}

/// The raw descriptors of `table`, unpacked and checked against pandecode's
/// fields: (sampler, texture, planes).
fn table(desc: &str, file: &str, table: &str) -> (Sampler, Texture, Vec<AfbcPlane>) {
    let lines: Vec<&str> = desc.lines().collect();
    let raw = raws(desc);
    let of = |kind: &str| {
        raw.iter()
            .find(|r| r.table == table && r.kind == kind)
            .unwrap_or_else(|| panic!("{file}: no {table} {kind} words"))
    };

    let rs = of("Sampler");
    let s = Sampler::unpack(&rs.bytes).unwrap_or_else(|e| panic!("{file}: {table} sampler: {e:?}"));
    let (_, f) = decoded(&lines, table, rs.entry, "Sampler");
    assert_eq!(s, sampler_of(f), "{file}: the {table} sampler's words are not its decoded fields");
    assert_eq!(s.pack().unwrap(), rs.bytes, "{file}: the {table} sampler does not pack back");

    let rt = of("Texture");
    let t = Texture::unpack(&rt.bytes).unwrap_or_else(|e| panic!("{file}: {table} texture: {e:?}"));
    assert_eq!(t.pack().unwrap(), rt.bytes, "{file}: the {table} texture does not pack back");
    let (head, mut f) = decoded(&lines, table, rt.entry, "Texture");
    let f = &mut f;
    assert_eq!(take(f, "Type"), "Texture");
    assert_eq!(take(f, "Dimension"), "Cube");
    assert_eq!(t.type_, DescriptorType::TEXTURE);
    assert_eq!(t.dimension, TextureDimension::CUBE);
    assert_eq!(t.sample_corner_position, flag(f, "Sample corner position"));
    assert_eq!(t.normalize_coordinates, flag(f, "Normalize coordinates"));
    assert_eq!(u64::from(t.width), num(f, "Width"), "{file}: width");
    assert_eq!(u64::from(t.height), num(f, "Height"), "{file}: height");
    assert_eq!(u64::from(t.swizzle), num(f, "Swizzle"), "{file}: swizzle");
    assert_eq!(t.texel_interleave, flag(f, "Texel interleave"));
    assert_eq!(u64::from(t.levels), num(f, "Levels"), "{file}: levels");
    assert_eq!(u64::from(t.minimum_level), num(f, "Minimum level"));
    assert_eq!(t.minimum_lod, take(f, "Minimum LOD").parse::<f32>().unwrap());
    assert_eq!(u64::from(t.sample_count), num(f, "Sample count"));
    assert_eq!(t.maximum_lod, take(f, "Maximum LOD").parse::<f32>().unwrap());
    assert_eq!(t.surfaces, num(f, "Surfaces"), "{file}: surfaces");
    assert_eq!(u64::from(t.array_size), num(f, "Array size"));
    assert_eq!(u64::from(t.depth), num(f, "Depth"));
    assert!(take(f, "Format (v7)").contains("UNORM"), "{file}: format");
    assert!(f.is_empty(), "{file}: texture fields not read: {f:?}");

    // the planes: Levels x 6 faces, each decoded below the texture
    assert_eq!(rt.planes.len() as u32, t.levels * 6, "{file}: {table} planes");
    let mut planes = Vec::new();
    for (k, b) in rt.planes.iter().enumerate() {
        let p = AfbcPlane::unpack(b).unwrap_or_else(|e| panic!("{file}: plane {k}: {e:?}"));
        assert_eq!(p.pack().unwrap(), *b, "{file}: plane {k} does not pack back");
        let at = head
            + lines[head..]
                .iter()
                .position(|l| l.trim() == format!("Plane {k}:"))
                .unwrap_or_else(|| panic!("{file}: decoded plane {k}"));
        let mut f = fields_below(&lines, at);
        let f = &mut f;
        assert_eq!((take(f, "Type"), take(f, "Plane type")), ("Plane".into(), "AFBC".into()));
        assert_eq!((p.type_, p.plane_type), (DescriptorType::PLANE, PlaneType::AFBC));
        assert_eq!(p.ytr, flag(f, "YTR"));
        assert_eq!(p.split_block, flag(f, "Split block"));
        assert_eq!(p.alpha_hint, flag(f, "Alpha hint"));
        assert_eq!(p.tiled_header, flag(f, "Tiled header"));
        assert_eq!(p.prefetch, flag(f, "Prefetch"));
        assert_eq!(u64::from(p.size), num(f, "Size"), "{file}: plane {k} size");
        assert_eq!(p.pointer, num(f, "Pointer"), "{file}: plane {k} pointer");
        assert_eq!(u64::from(p.header_row_stride), num(f, "Header row stride"));
        assert_eq!(u64::from(p.header_slice_size), num(f, "Header slice size"));
        assert_eq!(u64::from(p.header_slice_stride), num(f, "Header slice stride"));
        assert_eq!(u64::from(p.width), num(f, "Width"), "{file}: plane {k} width");
        assert_eq!(u64::from(p.height), num(f, "Height"), "{file}: plane {k} height");
        take(f, "Superblock size");
        take(f, "Compression mode");
        assert!(f.is_empty(), "{file}: plane {k} fields not read: {f:?}");
        planes.push(p);
    }
    assert_eq!(rt.va, u64::from_str_radix(lines[head - 1].trim().trim_start_matches("Texture @"), 16).unwrap());
    (s, t, planes)
}

/* ---- the descriptors ------------------------------------------------------- */

#[test]
fn descriptor_words_are_the_decoded_fields_and_the_case_s_state() {
    for c in CASES {
        let desc = testdata(c.file);
        let (name, ctx) = c.file.trim_end_matches(".desc").rsplit_once('.').unwrap();
        let title = desc.lines().next().unwrap();
        assert!(title.starts_with(&format!("# {name}: ")) && title.ends_with(&format!("; context {ctx}")), "{title}");
        assert_eq!(ctx == "es2", c.es2, "{}", c.file);
        let version = if c.es2 { "OpenGL ES 2.0 Mesa 26.2.1" } else { "OpenGL ES 3.1 Mesa 26.2.1" };
        assert!(
            desc.lines().any(|l| l == format!("# vs_cube_edge_draw: GL_VERSION {version}")),
            "{}: GL_VERSION {version}",
            c.file
        );
        let mut first: Option<(Sampler, Texture)> = None;
        for t in c.tables {
            let (s, tex, planes) = table(&desc, c.file, t);
            // the stage's sampler is the case's GL state
            let (wrap, seamless) = if c.es2 { (WrapMode::REPEAT, false) } else { (WrapMode::CLAMP_TO_EDGE, true) };
            assert_eq!((s.wrap_mode_r, s.wrap_mode_t, s.wrap_mode_s), (wrap, wrap, wrap), "{}: {t} wraps", c.file);
            assert_eq!(s.seamless_cube_map, seamless, "{}: {t} seamless", c.file);
            assert!(!s.minify_nearest, "{}: min filter LINEAR_MIPMAP_*", c.file);
            assert_eq!(s.magnify_nearest, c.mag_nearest, "{}: mag filter", c.file);
            assert_eq!(s.mipmap_mode, c.mipmap, "{}: mipmap mode", c.file);
            assert!(s.normalized_coordinates && !s.magnify_cutoff, "{}", c.file);
            assert_eq!((s.minimum_lod, s.lod_bias), (0.0, 0.0), "{}", c.file);
            // the texture is the case's cube map
            assert_eq!(
                (tex.width, tex.height, tex.levels, tex.array_size),
                (c.size, c.size, c.levels, 1),
                "{}",
                c.file
            );
            // the planes are face-major: each face's levels in turn
            for (k, p) in planes.iter().enumerate() {
                let level = k as u32 % c.levels;
                let n = (c.size >> level).max(1);
                assert_eq!((p.width, p.height), (n, n), "{}: plane {k} (level {level})", c.file);
            }
            // the Position and Varying tables of a vertex look-up are one
            match &first {
                None => first = Some((s, tex)),
                Some((s0, t0)) => assert_eq!((&s, &tex), (s0, t0), "{}: {t} differs from {}", c.file, c.tables[0]),
            }
        }
    }
}

#[test]
fn the_contexts_differ_in_seamless_and_wraps_only() {
    for pair in CASES.windows(2).filter(|p| !p[0].es2 && p[1].es2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert_eq!(a.file.replace(".es3.", ".es2."), b.file);
        let (da, db) = (testdata(a.file), testdata(b.file));
        let (sa, ta, pa) = table(&da, a.file, a.tables[0]);
        let (mut sb, tb, pb) = table(&db, b.file, b.tables[0]);
        assert_eq!((ta, pa), (tb, pb), "{}: the texture depends on the context", b.file);
        assert_ne!(sa, sb);
        sb.seamless_cube_map = sa.seamless_cube_map;
        (sb.wrap_mode_r, sb.wrap_mode_t, sb.wrap_mode_s) = (sa.wrap_mode_r, sa.wrap_mode_t, sa.wrap_mode_s);
        assert_eq!(sa, sb, "{}: the samplers differ in more than seamless and the wraps", b.file);
    }
}

#[test]
fn the_raw_words_hold_the_known_sampler_words() {
    // word 0 of the stage's sampler, as committed: the context and the
    // magnification filter are all that moves it
    let want = [
        ("vs_cube_edge.lml_linear_repeat.edge.es3.desc", 0xc6a9_9901u32),
        ("vs_cube_edge.lml_linear_repeat.edge.es2.desc", 0xc628_8801),
        ("vs_cube_edge.lml_nearest_repeat.edge.es3.desc", 0xd6a9_9901),
        ("vs_cube_edge.lml_nearest_repeat.edge.es2.desc", 0xd628_8801),
        ("cube_lod_vs.lmn_linear_repeat.es2.desc", 0x0628_8801),
    ];
    for (file, w0) in want {
        let r = raws(&testdata(file));
        let s = r.iter().find(|r| r.table == "Position" && r.kind == "Sampler").expect("a Position sampler");
        assert_eq!(u32::from_le_bytes(s.bytes[0..4].try_into().unwrap()), w0, "{file}");
    }
}

/* ---- the programs ---------------------------------------------------------- */

/// The `text` lines of `region` in an `.expect.json`, checked against its
/// `words` (one 64-bit hex word per line).
fn region_text(json: &str, file: &str, region: &str) -> Vec<String> {
    let r = &json[json.find(&format!("\"{region}\": {{")).unwrap_or_else(|| panic!("{file}: {region}"))..];
    let list = |key: &str| -> Vec<String> {
        let at = r.find(&format!("\"{key}\": [")).unwrap_or_else(|| panic!("{file}: {region}.{key}")) + key.len() + 5;
        let body = &r[at..at + r[at..].find("\n   ]").expect("the end of the list")];
        body.lines()
            .map(|l| l.trim().trim_end_matches(','))
            .filter(|l| !l.is_empty())
            .map(|l| l.trim_matches('"').to_string())
            .collect()
    };
    let words = list("words");
    let text = list("text");
    assert!(!words.is_empty(), "{file}: {region} has no words");
    assert_eq!(words.len(), text.len(), "{file}: {region} words and text");
    for w in &words {
        assert!(w.len() == 16 && u64::from_str_radix(w, 16).is_ok(), "{file}: {region} word {w}");
    }
    assert!(r.contains("\"found\": true"), "{file}: {region} not found in the trace");
    text
}

/// Mesa's cube look-up: the face selection, the face coordinates and one
/// explicit-LOD cube TEX_SINGLE.
fn has_cube_look_up(text: &[String]) -> bool {
    let at = |op: &str| text.iter().position(|l| l.starts_with(op));
    let (Some(f1), Some(f2), Some(s), Some(t)) =
        (at("CUBEFACE1"), at("CUBEFACE2_V9"), at("CUBE_SSEL"), at("CUBE_TSEL"))
    else {
        return false;
    };
    let tex: Vec<&String> = text.iter().filter(|l| l.starts_with("TEX")).collect();
    tex.len() == 1
        && tex[0].starts_with("TEX_SINGLE")
        && tex[0].contains(".cube.explicit")
        && f1 < f2
        && f2 < s
        && s < t
        && t < text.iter().position(|l| l.starts_with("TEX")).unwrap()
}

#[test]
fn the_programs_sample_the_cube_in_the_case_s_stage() {
    for (prog, stage) in [("vs_cube_edge", "vs.varying"), ("cube_lod_vs", "vs.varying"), ("cube_lod_fs", "fs.fragment")]
    {
        let file = format!("{prog}.expect.json");
        let json = testdata(&file);
        assert!(json.contains(&format!("\"trace\": \"{prog}.trace\"")), "{file}: trace");
        for region in ["vs.position", "vs.varying", "fs.fragment"] {
            let text = region_text(&json, &file, region);
            assert_eq!(has_cube_look_up(&text), region == stage, "{file}: {region}");
        }
    }
}

#[test]
fn vs_cube_edge_is_pan_tex_ref_s_vcube() {
    for ext in ["vert", "frag"] {
        assert_eq!(testdata(&format!("vs_cube_edge.{ext}")), testdata(&format!("pan_tex_ref/vcube.{ext}")));
    }
    let ours = testdata("vs_cube_edge.expect.json").replacen("vs_cube_edge.trace", "vcube.trace", 1);
    assert_eq!(ours, testdata("pan_tex_ref/vcube.expect.json"));
}

#[test]
fn the_shaders_are_the_deqp_cases() {
    let vs = testdata("cube_lod_vs.vert");
    assert!(vs.contains("\tv_color = textureCubeLod(u_sampler, a_in0, a_in1);\n"));
    assert!(vs.contains("uniform lowp samplerCube u_sampler;\n"));
    let fs = testdata("cube_lod_fs.frag");
    assert!(fs.starts_with("#version 300 es\n"));
    assert!(fs.contains("textureLod(u_sampler, v_texCoord, v_lodBias)"));
    assert!(testdata("vs_cube_edge.vert").contains("textureCubeLod(u_texture, a_texCoord, u_lod)"));
}
