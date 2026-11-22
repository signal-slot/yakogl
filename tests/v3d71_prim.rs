// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/prim71.rs` (V3D 7.1 draw packets) against Mesa 26.2.1's
//! v3d_packet.xml and the draw packet Mesa emitted on the Pi 5
//! (tools/v3d/pi5/gl_viewport.clif: a decoded VERTEX_ARRAY_PRIMS block).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_prim
//! ```
#[path = "../src/v3d/prim71.rs"]
mod prim71;

use prim71::*;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn attr(line: &str, k: &str) -> Option<String> {
    let p = line.find(&format!("{k}=\""))? + k.len() + 2;
    Some(line[p..p + line[p..].find('"').unwrap()].to_string())
}

/// `(code, [(field, start, size)])` of the xml packet `name`.
fn xml_packet(name: &str) -> (u8, Vec<(String, u32, u32)>) {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let s = xml.find(&format!("name=\"{name}\"")).unwrap_or_else(|| panic!("{name}"));
    let s = xml[..s].rfind("<packet").unwrap();
    let body = &xml[s..s + xml[s..].find("</packet>").unwrap()];
    assert!(!body.contains("min_ver") && !body.contains("max_ver"), "{name}: version-specific");
    let code = attr(body.lines().next().unwrap(), "code").unwrap().parse().unwrap();
    let fields = body
        .lines()
        .filter(|l| l.trim_start().starts_with("<field "))
        .map(|l| {
            (
                attr(l, "name").unwrap(),
                attr(l, "start").unwrap().parse().unwrap(),
                attr(l, "size").unwrap().parse().unwrap(),
            )
        })
        .collect();
    (code, fields)
}

/// Pack field values by the XML's own layout (independent of prim71's code).
fn xml_pack(name: &str, vals: &[(&str, u64)]) -> Vec<u8> {
    let (code, fields) = xml_packet(name);
    let bits = fields.iter().map(|f| f.1 + f.2).max().unwrap();
    let mut b = vec![0u8; 1 + (bits as usize).div_ceil(8)];
    b[0] = code;
    for (n, v) in vals {
        let (_, st, sz) = fields.iter().find(|f| f.0 == *n).unwrap_or_else(|| panic!("{n}"));
        assert!(sz >= &64 || v >> sz == 0);
        for i in 0..*sz {
            if (v >> i) & 1 != 0 {
                b[1 + ((st + i) / 8) as usize] |= 1 << ((st + i) % 8);
            }
        }
    }
    b
}

#[test]
fn fields_match_the_xml() {
    let mut names: Vec<&str> = FIELDS.iter().map(|f| f.0).collect();
    names.dedup();
    assert_eq!(names.len(), 6);
    for pk in names {
        let (_, xf) = xml_packet(pk);
        let ours: Vec<_> = FIELDS.iter().filter(|f| f.0 == pk).collect();
        assert_eq!(ours.len(), xf.len(), "{pk}");
        for f in ours {
            assert!(xf.contains(&(f.1.to_string(), f.2, f.3)), "{pk}.{}", f.1);
        }
    }
    for (n, c) in [
        ("Indexed Prim List", OP_INDEXED_PRIM_LIST),
        ("Indexed Instanced Prim List", OP_INDEXED_INSTANCED_PRIM_LIST),
        ("Vertex Array Prims", OP_VERTEX_ARRAY_PRIMS),
        ("Vertex Array Instanced Prims", OP_VERTEX_ARRAY_INSTANCED_PRIMS),
        ("Base Vertex Base Instance", OP_BASE_VERTEX_BASE_INSTANCE),
        ("Index Buffer Setup", OP_INDEX_BUFFER_SETUP),
    ] {
        assert_eq!(xml_packet(n).0, c, "{n}");
    }
}

#[test]
fn primitive_enum_matches_the_xml() {
    let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
    let s = xml.find("<enum name=\"Primitive\"").unwrap();
    let body = &xml[s..s + xml[s..].find("</enum>").unwrap()];
    let val = |n: &str| -> u32 {
        let l = body.lines().find(|l| attr(l, "name").as_deref() == Some(n)).unwrap();
        attr(l, "value").unwrap().parse().unwrap()
    };
    for (n, g) in [
        ("POINTS", GL_POINTS),
        ("LINES", GL_LINES),
        ("LINE_LOOP", GL_LINE_LOOP),
        ("LINE_STRIP", GL_LINE_STRIP),
        ("TRIANGLES", GL_TRIANGLES),
        ("TRIANGLE_STRIP", GL_TRIANGLE_STRIP),
        ("TRIANGLE_FAN", GL_TRIANGLE_FAN),
    ] {
        assert_eq!(val(n), g, "{n}");
        assert_eq!(prim_mode(g), Ok(g as u8));
    }
}

/// The values of the first `VERTEX_ARRAY_PRIMS` block of a Mesa CLIF dump.
fn clif_array_prims(rel: &str) -> (u64, u64, u64) {
    let t = read(rel);
    let mut it = t.lines().skip_while(|l| !l.ends_with("info: VERTEX_ARRAY_PRIMS"));
    it.next().unwrap();
    let mut v = [0u64; 3];
    for (i, key) in ["mode:", "Length:", "Index of First Vertex:"].iter().enumerate() {
        let l = it.next().unwrap();
        assert!(l.contains(key), "{l}");
        let r = &l[l.find("*/").unwrap() + 2..];
        v[i] = r.split_whitespace().next().unwrap().parse().unwrap();
    }
    (v[0], v[1], v[2])
}

#[test]
fn triangle_draw_matches_the_pi5_reference() {
    let (mode, len, first) = clif_array_prims("tools/v3d/pi5/gl_viewport.clif");
    assert_eq!((mode, len, first), (6, 4, 0)); // a TRIANGLE_FAN quad
    let got = pack_array(mode as u32, first, len, 1).unwrap();
    let want = xml_pack("Vertex Array Prims", &[("mode", mode), ("Length", len), ("Index of First Vertex", first)]);
    assert_eq!(got, want);
    assert_eq!(got, [36, 6, 4, 0, 0, 0, 0, 0, 0, 0]);
    // The same through pack_draw, and a triangle list with a first vertex.
    assert_eq!(pack_draw(&Draw::arrays(GL_TRIANGLE_FAN, 0, 4)).unwrap(), got);
    assert_eq!(pack_draw(&Draw::arrays(GL_TRIANGLES, 3, 6)).unwrap(), [36, 4, 6, 0, 0, 0, 3, 0, 0, 0]);
    // every other reference with an array draw agrees with the packer
    for f in ["triangle", "gl_tex2d", "gl_scissor"] {
        let rel = format!("tools/v3d/pi5/{f}.clif");
        if read(&rel).contains("info: VERTEX_ARRAY_PRIMS") {
            let (m, l, s) = clif_array_prims(&rel);
            assert_eq!(
                pack_array(m as u32, s, l, 1).unwrap(),
                xml_pack("Vertex Array Prims", &[("mode", m), ("Length", l), ("Index of First Vertex", s)]),
                "{f}"
            );
        }
    }
}

#[test]
fn every_mode_packs() {
    for m in GL_POINTS..=GL_TRIANGLE_FAN {
        let p = pack_array(m, 7, 9, 1).unwrap();
        assert_eq!(
            p,
            xml_pack("Vertex Array Prims", &[("mode", m as u64), ("Length", 9), ("Index of First Vertex", 7)])
        );
    }
}

#[test]
fn instanced_array() {
    let p = pack_array(GL_TRIANGLE_STRIP, 2, 5, 10).unwrap();
    assert_eq!(
        p,
        xml_pack(
            "Vertex Array Instanced Prims",
            &[("mode", 5), ("Instance Length", 5), ("Number of Instances", 10), ("Index of First Vertex", 2)]
        )
    );
    assert_eq!(p.len(), 14);
}

#[test]
fn indexed_all_types() {
    for (gl, ty, bytes) in [(GL_UNSIGNED_BYTE, 0u64, 1u64), (GL_UNSIGNED_SHORT, 1, 2), (GL_UNSIGNED_INT, 2, 4)] {
        for restart in [false, true] {
            let ix = Indices { gl_type: gl, addr: 0x1000, size: 4096, offset: 8 * bytes, restart };
            let d = Draw {
                mode: GL_TRIANGLES,
                indices: Some(ix),
                first: 99,
                count: 30,
                instances: 1,
                base_vertex: 0,
                base_instance: 0,
            };
            let mut want = xml_pack("Index Buffer Setup", &[("Address", 0x1000), ("Size", 4096)]);
            want.extend(xml_pack(
                "Indexed Prim List",
                &[
                    ("mode", 4),
                    ("Index type", ty),
                    ("Length", 30),
                    ("Enable Primitive Restarts", restart as u64),
                    ("Index Offset", 8 * bytes),
                ],
            ));
            assert_eq!(pack_draw(&d).unwrap(), want, "{gl:x} {restart}");
        }
    }
}

#[test]
fn indexed_instanced_with_base() {
    let ix = Indices { gl_type: GL_UNSIGNED_SHORT, addr: 0x2000, size: 100, offset: 4, restart: true };
    let d = Draw {
        mode: GL_LINE_LOOP,
        indices: Some(ix),
        first: 0,
        count: 10,
        instances: 3,
        base_vertex: -5,
        base_instance: 2,
    };
    let mut want = xml_pack("Base Vertex Base Instance", &[("Base Vertex", 0xffff_fffb), ("Base Instance", 2)]);
    want.extend(xml_pack("Index Buffer Setup", &[("Address", 0x2000), ("Size", 100)]));
    want.extend(xml_pack(
        "Indexed Instanced Prim List",
        &[
            ("mode", 2),
            ("Index type", 1),
            ("Instance Length", 10),
            ("Enable Primitive Restarts", 1),
            ("Number of Instances", 3),
            ("Index Offset", 4),
        ],
    ));
    assert_eq!(pack_draw(&d).unwrap(), want);
}

#[test]
fn refusals() {
    let r = |d: Draw| pack_draw(&d).unwrap_err();
    assert_eq!(r(Draw::arrays(7, 0, 3)), Refusal71::Mode(7)); // GL_LINES_ADJACENCY region / quads
    assert_eq!(r(Draw::arrays(0xe, 0, 3)), Refusal71::Mode(0xe)); // GL_PATCHES
    assert_eq!(r(Draw::arrays(16, 0, 3)), Refusal71::Mode(16)); // POINTS_TF
    assert_eq!(r(Draw::arrays(4, 0, 0)), Refusal71::ZeroCount);
    let mut d = Draw::arrays(4, 0, 3);
    d.instances = 0;
    assert_eq!(r(d), Refusal71::ZeroInstances);
    assert_eq!(
        r(Draw::arrays(4, 1 << 32, 3)),
        Refusal71::Overflow { field: "Index of First Vertex", value: 1 << 32, bits: 32 }
    );
    assert_eq!(r(Draw::arrays(4, 0, 1 << 32)), Refusal71::Overflow { field: "Length", value: 1 << 32, bits: 32 });
    let mut d = Draw::arrays(4, 0, 3);
    d.instances = 1 << 32;
    assert_eq!(r(d), Refusal71::Overflow { field: "Number of Instances", value: 1 << 32, bits: 32 });
    let mut d = Draw::arrays(4, 0, 3);
    d.base_vertex = 1 << 31;
    assert_eq!(r(d), Refusal71::BaseVertex(1 << 31));
    d.base_vertex = i64::MIN;
    assert_eq!(r(d), Refusal71::BaseVertex(i64::MIN));
    let mut d = Draw::arrays(4, 0, 3);
    d.base_instance = 1 << 33;
    assert_eq!(r(d), Refusal71::Overflow { field: "Base Instance", value: 1 << 33, bits: 32 });

    let ix = |t, off, size| Some(Indices { gl_type: t, addr: 0, size, offset: off, restart: false });
    let el =
        |indices, count| Draw { mode: 4, indices, first: 0, count, instances: 1, base_vertex: 0, base_instance: 0 };
    assert_eq!(r(el(ix(0x1402, 0, 100), 3)), Refusal71::IndexType(0x1402)); // GL_SHORT
    assert_eq!(r(el(ix(GL_UNSIGNED_SHORT, 1, 100), 3)), Refusal71::IndexAlign { offset: 1, index_bytes: 2 });
    assert_eq!(r(el(ix(GL_UNSIGNED_SHORT, 0, 5), 3)), Refusal71::IndexRange { end: 6, size: 5 });
    assert_eq!(
        r(el(ix(GL_UNSIGNED_BYTE, 0, u32::MAX as u64), 1 << 31)),
        Refusal71::Overflow { field: "Length", value: 1 << 31, bits: 31 }
    );
    assert_eq!(r(el(ix(GL_UNSIGNED_BYTE, 0, 100), 0)), Refusal71::ZeroCount);
    // unreachable through pack_draw (the size overflows first): the packer itself
    assert_eq!(
        pack_indexed(4, GL_UNSIGNED_BYTE, 1 << 32, 3, 1, false).unwrap_err(),
        Refusal71::Overflow { field: "Index Offset", value: 1 << 32, bits: 32 }
    );
    assert_eq!(
        r(el(ix(GL_UNSIGNED_BYTE, 0, 1 << 32), 3)),
        Refusal71::Overflow { field: "Size", value: 1 << 32, bits: 32 }
    );
    assert_eq!(r(el(ix(GL_UNSIGNED_INT, 0, 8), u64::MAX)), Refusal71::IndexRange { end: u64::MAX, size: 8 });
}

#[test]
fn refusals_display_by_name() {
    let all = [
        Refusal71::Mode(7),
        Refusal71::IndexType(1),
        Refusal71::ZeroCount,
        Refusal71::ZeroInstances,
        Refusal71::Overflow { field: "Length", value: 1, bits: 2 },
        Refusal71::BaseVertex(1),
        Refusal71::IndexRange { end: 1, size: 0 },
        Refusal71::IndexAlign { offset: 1, index_bytes: 2 },
    ];
    for r in all {
        assert!(r.to_string().starts_with("7.1 draw: "), "{r}");
    }
}
