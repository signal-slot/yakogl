// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Golden input/output pairs for the GLSL front end. The expected texts were
//! produced by the Python original (upstream c1e6084 tools/glsl2usc.py); each
//! case exercises a place where a naive regex port goes wrong.

use super::*;

fn names(n: &GlNames) -> Vec<String> {
    let mut m: Vec<String> = Vec::new();
    let mut members: Vec<_> = n.members.iter().collect();
    members.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in members {
        m.push(match v {
            GlName::Name(s) => format!("{k} -> {s}"),
            GlName::Inner(s, i) => format!("{k} -> {s} inner {i}"),
        });
    }
    let mut s: Vec<_> = n.samplers.iter().collect();
    s.sort();
    m.extend(s.into_iter().map(|(k, v)| format!("sampler {k} -> 0x{v:x}")));
    m
}

fn uniforms(text: &str, n: &GlNames) -> (i64, Vec<String>) {
    let (u, dw) = reflect_uniforms(text, n).unwrap();
    let v = u
        .iter()
        .map(|u| {
            format!(
                "{} off={} dwords={} type=0x{:x} len={} binding={} inner={}",
                u.name, u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner
            )
        })
        .collect();
    (dw, v)
}

fn exit_msg<T: std::fmt::Debug>(r: R<T>) -> String {
    match r {
        Err(Error::Exit(m)) => m,
        other => panic!("expected an input error, got {other:?}"),
    }
}

#[test]
fn es100_declarations_and_hoist() {
    // attributes get locations by size, varyings are numbered separately, int varyings become flat,
    // a comment's bidi mark is stripped, gl_Position is hoisted before `v_uv = uv`
    let src = "#version 100\n// comment with \u{e9} and a \u{202e} bidi mark\nattribute vec2 uv; attribute highp vec4 pos;\nattribute float w[2];\ninvariant varying mediump vec2 v_uv;\nvarying ivec2 v_i, v_j[3];\nuniform mat4 mvp;\nvoid main() {\n  v_uv = uv;\n  gl_Position = mvp * pos;\n}\n";
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"vs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
layout(location = 0) in vec2 uv;
layout(location = 1) in vec4 pos;
layout(location = 2) in float w;
invariant layout(location = 0) out vec2 v_uv;
layout(location = 1) flat out ivec2 v_i;
layout(location = 2) flat out ivec2 v_j[3];
layout(push_constant) uniform pvr_U {
	mat4 mvp;
};

void main() {  gl_Position = mvp * pos;

  v_uv = uv;
}"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            16,
            vec![r#"mvp off=0 dwords=16 type=0x8b5c len=1 binding=0 inner=1"#]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"v_uv"#.to_string()), (1, r#"v_i"#.to_string()), (2, r#"v_j"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![1, 2];
    assert_eq!(f, e);
    let mut a: Vec<_> = reflect_attribs(&text).unwrap().into_iter().collect();
    a.sort();
    let e: Vec<(i64, (String, i64))> = vec![
        (0, (r#"uv"#.to_string(), 0x8b50)),
        (1, (r#"pos"#.to_string(), 0x8b52)),
        (2, (r#"w"#.to_string(), 0x1406)),
    ];
    assert_eq!(a, e);
}

#[test]
fn es100_fragment() {
    // the unused sampler is dropped; gl_FragColor becomes the location-0 output
    let src = r#"precision mediump float;
invariant varying mediump vec2 v_uv;
varying ivec2 v_i;
uniform sampler2D tex;
uniform sampler2D unused_tex;
uniform highp vec4 col[ 4 ];
void main() { gl_FragColor = texture2D(tex, v_uv) * col[1]; }
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
precision mediump float;layout(location = 0) in vec2 v_uv;
layout(location = 1) flat in ivec2 v_i;
layout(set = 0, binding = 0) uniform sampler2D tex;
layout(push_constant) uniform pvr_U {
	vec4 col[4];
};

void main() { gl_FragColor = texture2D(tex, v_uv) * col[1]; }"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            16,
            vec![
                r#"col off=0 dwords=16 type=0x8b52 len=4 binding=0 inner=1"#,
                r#"tex off=65535 dwords=1 type=0x8b5e len=1 binding=0 inner=1"#
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"v_uv"#.to_string()), (1, r#"v_i"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![1];
    assert_eq!(f, e);
}

#[test]
fn hoist_blocked_by_a_local() {
    // the gl_Position RHS reads a local assigned earlier: no reorder
    let src = r#"attribute vec4 pos; attribute vec2 uv;
varying vec2 v;
void main() {
  vec4 p = pos;
  v = uv;
  gl_Position = p;
}
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"vs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
layout(location = 0) in vec4 pos;
layout(location = 1) in vec2 uv;
layout(location = 0) out vec2 v;

void main() {
  vec4 p = pos;
  v = uv;
  gl_Position = p;
}"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(uniforms(&text, &n), (0, Vec::<String>::new()));
    let mut v: Vec<_> = reflect_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"v"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
    let mut a: Vec<_> = reflect_attribs(&text).unwrap().into_iter().collect();
    a.sort();
    let e: Vec<(i64, (String, i64))> = vec![(0, (r#"pos"#.to_string(), 0x8b52)), (1, (r#"uv"#.to_string(), 0x8b50))];
    assert_eq!(a, e);
}

#[test]
fn hoist_takes_trailing_blanks() {
    // the moved statement takes its trailing blanks and newline; a later write stays
    let src = r#"attribute vec4 pos; attribute vec2 uv;
varying vec2 v;
void main()
{
	v = uv;
	gl_Position = pos;  
	gl_Position.x += 0.0;
}
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"vs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
layout(location = 0) in vec4 pos;
layout(location = 1) in vec2 uv;
layout(location = 0) out vec2 v;

void main()
{	gl_Position = pos;  

	v = uv;
	gl_Position.x += 0.0;
}"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(uniforms(&text, &n), (0, Vec::<String>::new()));
    let mut v: Vec<_> = reflect_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"v"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
    let mut a: Vec<_> = reflect_attribs(&text).unwrap().into_iter().collect();
    a.sort();
    let e: Vec<(i64, (String, i64))> = vec![(0, (r#"pos"#.to_string(), 0x8b52)), (1, (r#"uv"#.to_string(), 0x8b50))];
    assert_eq!(a, e);
}

#[test]
fn struct_flattening() {
    // nested structs, an array of structs with an inner array (a.b[i].c[j] -> flat[(i)*M+(j)]),
    // component indexing on a vec member (a.b[i].v[k] -> flat[i][k]) and spaced member access
    let src = r#"#version 100
precision highp float;
#define NL 2
struct Light { vec3 dir; vec4 col[2]; float k; };
struct Inner { float a; vec2 b; };
struct Mid { Inner in1; mat3 rot; };
struct Buf { mat4 m; Mid mid; Light lights[NL]; vec3 amb[2]; };
uniform Buf ubuf;
void main() {
  vec4 c = ubuf.lights[1].col[ 0 ] + ubuf . lights[ 0 ] . col[1];
  float k = ubuf.lights[0].k + ubuf.mid.in1.a + ubuf.mid . in1 . b.x;
  vec3 d = ubuf.lights[1].dir;
  float dx = ubuf.lights[1].dir[2];
  for (int i = 0; float(i) < float(NL); i++) { c += ubuf.lights[i].col[1] * ubuf.lights[i].k; }
  for (int j = 0; j < 2; j++) { c.rgb += ubuf.amb[j]; }
  gl_FragColor = c * k + vec4(d, dx) + vec4(ubuf.m[0]);
}
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
precision highp float;
#define NL 2
struct Light { vec3 dir; vec4 col[2]; float k; };
struct Inner { float a; vec2 b; };
struct Mid { Inner in1; mat3 rot; };
struct Buf { mat4 m; Mid mid; Light lights[NL]; vec3 amb[2]; };layout(push_constant) uniform pvr_U {
	mat4 ubuf_m;
	float ubuf_mid_in1_a;
	vec2 ubuf_mid_in1_b;
	mat3 ubuf_mid_rot;
	vec3 ubuf_lights_dir[2];
	vec4 ubuf_lights_col[4];
	float ubuf_lights_k[2];
	vec3 ubuf_amb[2];
};

void main() {
  vec4 c = ubuf_lights_col[(1)*2+(0)] + ubuf_lights_col[(0)*2+(1)];
  float k = ubuf_lights_k[0] + ubuf_mid_in1_a + ubuf_mid_in1_b.x;
  vec3 d = ubuf_lights_dir[1];
  float dx = ubuf_lights_dir[1][2];
  for (int i = 0; float(i) < float(NL); i++) { c += ubuf_lights_col[(i)*2+(1)] * ubuf_lights_k[i]; }
  for (int j = 0; j < 2; j++) { c.rgb += ubuf_amb[j]; }
  gl_FragColor = c * k + vec4(d, dx) + vec4(ubuf_m[0]);
}"#
    );
    let e: Vec<&str> = vec![
        r#"ubuf_amb -> ubuf.amb"#,
        r#"ubuf_lights_col -> ubuf.lights[0].col inner 2"#,
        r#"ubuf_lights_dir -> ubuf.lights[0].dir inner 1"#,
        r#"ubuf_lights_k -> ubuf.lights[0].k inner 1"#,
        r#"ubuf_m -> ubuf.m"#,
        r#"ubuf_mid_in1_a -> ubuf.mid.in1.a"#,
        r#"ubuf_mid_in1_b -> ubuf.mid.in1.b"#,
        r#"ubuf_mid_rot -> ubuf.mid.rot"#,
    ];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            68,
            vec![
                r#"ubuf.m off=0 dwords=16 type=0x8b5c len=1 binding=0 inner=1"#,
                r#"ubuf.mid.in1.a off=16 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"ubuf.mid.in1.b off=18 dwords=2 type=0x8b50 len=1 binding=0 inner=1"#,
                r#"ubuf.mid.rot off=20 dwords=12 type=0x8b5b len=1 binding=0 inner=1"#,
                r#"ubuf.lights[0].dir off=32 dwords=8 type=0x8b51 len=2 binding=0 inner=1"#,
                r#"ubuf.lights[0].col off=40 dwords=16 type=0x8b52 len=4 binding=0 inner=2"#,
                r#"ubuf.lights[0].k off=56 dwords=2 type=0x1406 len=2 binding=0 inner=1"#,
                r#"ubuf.amb off=60 dwords=8 type=0x8b51 len=2 binding=0 inner=1"#
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn emulated_samplers() {
    // cube/array samplers through 2D textures: globals, parameters, user functions taking array samplers
    // (including the original's argument-index quirks), nested calls and a call with no arguments
    let src = r#"#version 300 es
precision highp float;
uniform samplerCube env;
uniform lowp sampler2DArray layers;
uniform sampler2DArray layers2;
uniform sampler2DArrayShadow sh;
in vec3 dir;
layout(location = 1) out vec4 o1, o2;
out vec4 o3;
vec4 pick(highp sampler2DArray a, float l, samplerCube c) {
  return texture(a, vec3(0.5, 0.5, l)) + textureLod(c, vec3(1.0), 0.0) + vec4(textureSize(c, 0), 0.0, 0.0);
}
vec4 two(sampler2DArray a, sampler2DArray b) { return texture(a, vec3(0.0)) + texture(b, vec3(1.0)); }
vec4 lead(float pvr_layers_q, vec4 x) { return x; }
void main() {
  ivec3 s = textureSize(layers, 0);
  vec4 t = texelFetch(layers2, ivec3(0), 0) + texture(env, texture(layers, vec3(dir.xy, 1.0)).xyz);
  o1 = pick(layers, 2.0, env) + two(layers, layers2) + lead(1.0, t) + texture(sh, vec4(0.0));
  o2 = textureLod(layers, vec3(0.0), 1.0);
  o3 = vec4(textureSize());
}
"#;
    assert_eq!(es_version(src).unwrap(), Some(300));
    let (text, n) = translate(src, r#"fs"#, 300).unwrap();
    assert_eq!(
        text,
        r#"#version 450
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
#define gl_VertexID gl_VertexIndex
#define gl_InstanceID gl_InstanceIndex
precision highp float;layout(set = 0, binding = 0) uniform samplerCube env;
layout(set = 0, binding = 1) uniform sampler2D layers;
layout(push_constant) uniform pvr_U {
	float pvr_layers_layers;
	float pvr_layers_layers2;
};
layout(set = 0, binding = 2) uniform sampler2D layers2;
layout(set = 0, binding = 3) uniform sampler2DArrayShadow sh;
layout(location = 0) in vec3 dir;
layout(location = 1) out vec4 o1;
layout(location = 1) out vec4 o2;
layout(location = 2) out vec4 o3;


vec2 pvr_arrUV(sampler2D s, float layers, vec3 c) {
	float l = clamp(floor(c.z + 0.5), 0.0, layers - 1.0);
	float h = float(textureSize(s, 0).y) / layers;
	float v = clamp(c.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(c.x, (v + l) / layers);
}
vec4 pvr_arr(sampler2D s, float layers, vec3 c) { return texture(s, pvr_arrUV(s, layers, c)); }
vec4 pvr_arrLod(sampler2D s, float layers, vec3 c, float lod) { return textureLod(s, pvr_arrUV(s, layers, c), lod); }
vec4 pvr_arrFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int h = textureSize(s, lod).y / int(layers);
	return texelFetch(s, ivec2(c.x, c.y + c.z * h), lod);
}
ivec3 pvr_arrSize(sampler2D s, float layers, int lod) {
	ivec2 sz = textureSize(s, lod);
	return ivec3(sz.x, sz.y / int(layers), int(layers));
}
vec2 pvr_cubeUV(sampler2D s, vec3 d) {
	vec3 a = abs(d);
	float face, ma;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		ma = a.x;
		if (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
	} else if (a.y >= a.z) {
		ma = a.y;
		if (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
	} else {
		ma = a.z;
		if (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
	}
	uv = (uv / ma) * 0.5 + 0.5;
	float h = float(textureSize(s, 0).y) / 6.0;
	uv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(uv.x, (uv.y + face) / 6.0);
}
vec2 pvr_cubeUVLod(sampler2D s, vec3 d, float lod) {
	vec3 a = abs(d);
	float face, ma;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		ma = a.x;
		if (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
	} else if (a.y >= a.z) {
		ma = a.y;
		if (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
	} else {
		ma = a.z;
		if (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
	}
	uv = (uv / ma) * 0.5 + 0.5;
	float h = float(textureSize(s, 0).y) / 6.0;
	h = max(h * exp2(-ceil(max(lod, 0.0))), 1.0);
	uv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(uv.x, (uv.y + face) / 6.0);
}
vec4 pvr_cube(sampler2D s, vec3 d) { return texture(s, pvr_cubeUV(s, d)); }
vec4 pvr_cubeLod(sampler2D s, vec3 d, float lod) { return textureLod(s, pvr_cubeUVLod(s, d, lod), lod); }
ivec2 pvr_cubeSize(sampler2D s, int lod) { ivec2 sz = textureSize(s, lod); return ivec2(sz.x, sz.y / 6); }
vec2 pvr_volUV(sampler2D s, float layers, vec2 uv, float k) {
	float cols = ceil(sqrt(layers - 0.5));
	float rows = ceil(layers / cols);
	vec2 sz = vec2(textureSize(s, 0)) / vec2(cols, rows);
	vec2 st = clamp(uv, 0.5 / sz, 1.0 - 0.5 / sz);
	return (vec2(mod(k, cols), floor(k / cols)) + st) / vec2(cols, rows);
}
vec4 pvr_vol(sampler2D s, float layers, vec3 c) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(texture(s, pvr_volUV(s, layers, c.xy, z0)), texture(s, pvr_volUV(s, layers, c.xy, z1)), z - z0);
}
vec4 pvr_volLod(sampler2D s, float layers, vec3 c, float lod) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(textureLod(s, pvr_volUV(s, layers, c.xy, z0), 0.0), textureLod(s, pvr_volUV(s, layers, c.xy, z1), 0.0), z - z0);
}
vec4 pvr_volFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return texelFetch(s, c.xy + ivec2(c.z % cols, c.z / cols) * sz, 0);
}
ivec3 pvr_volSize(sampler2D s, float layers, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return ivec3(sz, int(layers));
}
vec4 pick(sampler2D a, float pvr_layers_a, float l, samplerCube c) {
  return pvr_arr(a, pvr_layers_a, vec3(0.5, 0.5, l)) + textureLod(c, vec3(1.0), 0.0) + vec4(textureSize(c, 0), 0.0, 0.0);
}
vec4 two(sampler2D a, float pvr_layers_a, sampler2D b, float pvr_layers_b) { return pvr_arr(a, pvr_layers_a, vec3(0.0)) + pvr_arr(b, pvr_layers_b, vec3(1.0)); }
vec4 lead(float pvr_layers_q, vec4 x) { return x; }
void main() {
  ivec3 s = pvr_arrSize(layers, pvr_layers_layers, 0);
  vec4 t = pvr_arrFetch(layers2, pvr_layers_layers2, ivec3(0), 0) + texture(env, pvr_arr(layers, pvr_layers_layers, vec3(dir.xy, 1.0)).xyz);
  o1 = pick(layers, pvr_layers_layers, 2.0, env) + two(layers, pvr_layers_layers, pvr_layers_pvr_layers_layers, layers2) + lead( pvr_layers_t,1.0, t) + texture(sh, vec4(0.0));
  o2 = pvr_arrLod(layers, pvr_layers_layers, vec3(0.0), 1.0);
  o3 = vec4(textureSize());
}"#
    );
    /* env is a real samplerCube in the 450 text (native cube step C): its
     * GL type is its 450 type, so it needs no mapping */
    let e: Vec<&str> = vec![r#"sampler layers -> 0x8dc1"#, r#"sampler layers2 -> 0x8dc1"#];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            2,
            vec![
                r#"pvr_layers_layers off=0 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"pvr_layers_layers2 off=1 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"env off=65535 dwords=1 type=0x8b60 len=1 binding=0 inner=1"#,
                r#"layers off=65535 dwords=1 type=0x8dc1 len=1 binding=1 inner=1"#,
                r#"layers2 off=65535 dwords=1 type=0x8dc1 len=1 binding=2 inner=1"#,
                r#"sh off=65535 dwords=1 type=0x8dc4 len=1 binding=3 inner=1"#
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"dir"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn sampler3d_emulation() {
    // sampler3D through a grid-tiled 2D texture (upstream c1e6084): globals with precision and a list,
    // parameters next to an array sampler, user functions taking two volumes, all four sampling calls
    let src = r#"#version 300 es
precision highp float;
uniform mediump sampler3D vol;
uniform sampler3D lut, unusedvol;
uniform sampler2DArray arr;
uniform sampler3D spare;
in vec3 p;
out vec4 o;
vec4 look(highp sampler3D v, vec3 c, sampler2DArray a) { return texture(v, c) + texture(a, c); }
vec4 both(sampler3D x, float k, sampler3D y) { return textureLod(x, vec3(k), 0.0) + texelFetch(y, ivec3(k), 0); }
void main() {
  ivec3 s = textureSize(vol, 0);
  vec4 f = texelFetch(lut, ivec3(1, 2, 3), 0) + textureLod(vol, p, 2.0);
  o = look(vol, texture(lut, p).xyz, arr) + both(lut, 1.0, vol) + vec4(textureSize(spare, 0), 1.0) + f;
}
"#;
    assert_eq!(es_version(src).unwrap(), Some(300));
    let (text, n) = translate(src, r#"fs"#, 300).unwrap();
    assert_eq!(
        text,
        r#"#version 450
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
#define gl_VertexID gl_VertexIndex
#define gl_InstanceID gl_InstanceIndex
precision highp float;layout(set = 0, binding = 0) uniform sampler2D vol;
layout(push_constant) uniform pvr_U {
	float pvr_layers_vol;
	float pvr_layers_lut;
	float pvr_layers_arr;
	float pvr_layers_spare;
};
layout(set = 0, binding = 1) uniform sampler2D lut;
layout(set = 0, binding = 2) uniform sampler2D arr;
layout(set = 0, binding = 3) uniform sampler2D spare;
layout(location = 0) in vec3 p;
layout(location = 0) out vec4 o;


vec2 pvr_arrUV(sampler2D s, float layers, vec3 c) {
	float l = clamp(floor(c.z + 0.5), 0.0, layers - 1.0);
	float h = float(textureSize(s, 0).y) / layers;
	float v = clamp(c.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(c.x, (v + l) / layers);
}
vec4 pvr_arr(sampler2D s, float layers, vec3 c) { return texture(s, pvr_arrUV(s, layers, c)); }
vec4 pvr_arrLod(sampler2D s, float layers, vec3 c, float lod) { return textureLod(s, pvr_arrUV(s, layers, c), lod); }
vec4 pvr_arrFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int h = textureSize(s, lod).y / int(layers);
	return texelFetch(s, ivec2(c.x, c.y + c.z * h), lod);
}
ivec3 pvr_arrSize(sampler2D s, float layers, int lod) {
	ivec2 sz = textureSize(s, lod);
	return ivec3(sz.x, sz.y / int(layers), int(layers));
}
vec2 pvr_cubeUV(sampler2D s, vec3 d) {
	vec3 a = abs(d);
	float face, ma;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		ma = a.x;
		if (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
	} else if (a.y >= a.z) {
		ma = a.y;
		if (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
	} else {
		ma = a.z;
		if (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
	}
	uv = (uv / ma) * 0.5 + 0.5;
	float h = float(textureSize(s, 0).y) / 6.0;
	uv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(uv.x, (uv.y + face) / 6.0);
}
vec2 pvr_cubeUVLod(sampler2D s, vec3 d, float lod) {
	vec3 a = abs(d);
	float face, ma;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		ma = a.x;
		if (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
	} else if (a.y >= a.z) {
		ma = a.y;
		if (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
	} else {
		ma = a.z;
		if (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
	}
	uv = (uv / ma) * 0.5 + 0.5;
	float h = float(textureSize(s, 0).y) / 6.0;
	h = max(h * exp2(-ceil(max(lod, 0.0))), 1.0);
	uv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(uv.x, (uv.y + face) / 6.0);
}
vec4 pvr_cube(sampler2D s, vec3 d) { return texture(s, pvr_cubeUV(s, d)); }
vec4 pvr_cubeLod(sampler2D s, vec3 d, float lod) { return textureLod(s, pvr_cubeUVLod(s, d, lod), lod); }
ivec2 pvr_cubeSize(sampler2D s, int lod) { ivec2 sz = textureSize(s, lod); return ivec2(sz.x, sz.y / 6); }
vec2 pvr_volUV(sampler2D s, float layers, vec2 uv, float k) {
	float cols = ceil(sqrt(layers - 0.5));
	float rows = ceil(layers / cols);
	vec2 sz = vec2(textureSize(s, 0)) / vec2(cols, rows);
	vec2 st = clamp(uv, 0.5 / sz, 1.0 - 0.5 / sz);
	return (vec2(mod(k, cols), floor(k / cols)) + st) / vec2(cols, rows);
}
vec4 pvr_vol(sampler2D s, float layers, vec3 c) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(texture(s, pvr_volUV(s, layers, c.xy, z0)), texture(s, pvr_volUV(s, layers, c.xy, z1)), z - z0);
}
vec4 pvr_volLod(sampler2D s, float layers, vec3 c, float lod) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(textureLod(s, pvr_volUV(s, layers, c.xy, z0), 0.0), textureLod(s, pvr_volUV(s, layers, c.xy, z1), 0.0), z - z0);
}
vec4 pvr_volFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return texelFetch(s, c.xy + ivec2(c.z % cols, c.z / cols) * sz, 0);
}
ivec3 pvr_volSize(sampler2D s, float layers, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return ivec3(sz, int(layers));
}
vec4 look(sampler2D v, float pvr_layers_v, vec3 c, sampler2D a, float pvr_layers_a) { return pvr_vol(v, pvr_layers_v, c) + pvr_arr(a, pvr_layers_a, c); }
vec4 both(sampler2D x, float pvr_layers_x, float k, sampler2D y, float pvr_layers_y) { return pvr_volLod(x, pvr_layers_x, vec3(k), 0.0) + pvr_volFetch(y, pvr_layers_y, ivec3(k), 0); }
void main() {
  ivec3 s = pvr_volSize(vol, pvr_layers_vol, 0);
  vec4 f = pvr_volFetch(lut, pvr_layers_lut, ivec3(1, 2, 3), 0) + pvr_volLod(vol, pvr_layers_vol, p, 2.0);
  o = look(vol, pvr_layers_vol, pvr_vol(lut, pvr_layers_lut, p).xyz, pvr_layers_pvr_vol(lut, pvr_layers_lut, p).xyz, arr) + both(lut, pvr_layers_lut, 1.0, pvr_layers_1.0, vol) + vec4(pvr_volSize(spare, pvr_layers_spare, 0), 1.0) + f;
}"#
    );
    let e: Vec<&str> = vec![
        r#"sampler arr -> 0x8dc1"#,
        r#"sampler lut -> 0x8b5f"#,
        r#"sampler spare -> 0x8b5f"#,
        r#"sampler vol -> 0x8b5f"#,
    ];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            4,
            vec![
                r#"pvr_layers_vol off=0 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"pvr_layers_lut off=1 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"pvr_layers_arr off=2 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"pvr_layers_spare off=3 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"vol off=65535 dwords=1 type=0x8b5f len=1 binding=0 inner=1"#,
                r#"lut off=65535 dwords=1 type=0x8b5f len=1 binding=1 inner=1"#,
                r#"arr off=65535 dwords=1 type=0x8dc1 len=1 binding=2 inner=1"#,
                r#"spare off=65535 dwords=1 type=0x8b5f len=1 binding=3 inner=1"#
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"p"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn sampler3d_in_es100() {
    // an ES 1.00 sampler3D is emulated too; texture2D on it is left alone
    let src = r#"precision mediump float;
uniform sampler3D tex;
varying vec3 v;
void main() {
  gl_FragColor = texture(tex, v) + texture2D(tex, v.xy);
}
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
precision mediump float;layout(set = 0, binding = 0) uniform sampler2D tex;
layout(push_constant) uniform pvr_U {
	float pvr_layers_tex;
};
layout(location = 0) in vec3 v;


vec2 pvr_arrUV(sampler2D s, float layers, vec3 c) {
	float l = clamp(floor(c.z + 0.5), 0.0, layers - 1.0);
	float h = float(textureSize(s, 0).y) / layers;
	float v = clamp(c.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(c.x, (v + l) / layers);
}
vec4 pvr_arr(sampler2D s, float layers, vec3 c) { return texture(s, pvr_arrUV(s, layers, c)); }
vec4 pvr_arrLod(sampler2D s, float layers, vec3 c, float lod) { return textureLod(s, pvr_arrUV(s, layers, c), lod); }
vec4 pvr_arrFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int h = textureSize(s, lod).y / int(layers);
	return texelFetch(s, ivec2(c.x, c.y + c.z * h), lod);
}
ivec3 pvr_arrSize(sampler2D s, float layers, int lod) {
	ivec2 sz = textureSize(s, lod);
	return ivec3(sz.x, sz.y / int(layers), int(layers));
}
vec2 pvr_cubeUV(sampler2D s, vec3 d) {
	vec3 a = abs(d);
	float face, ma;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		ma = a.x;
		if (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
	} else if (a.y >= a.z) {
		ma = a.y;
		if (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
	} else {
		ma = a.z;
		if (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
	}
	uv = (uv / ma) * 0.5 + 0.5;
	float h = float(textureSize(s, 0).y) / 6.0;
	uv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(uv.x, (uv.y + face) / 6.0);
}
vec2 pvr_cubeUVLod(sampler2D s, vec3 d, float lod) {
	vec3 a = abs(d);
	float face, ma;
	vec2 uv;
	if (a.x >= a.y && a.x >= a.z) {
		ma = a.x;
		if (d.x > 0.0) { face = 0.0; uv = vec2(-d.z, -d.y); } else { face = 1.0; uv = vec2(d.z, -d.y); }
	} else if (a.y >= a.z) {
		ma = a.y;
		if (d.y > 0.0) { face = 2.0; uv = vec2(d.x, d.z); } else { face = 3.0; uv = vec2(d.x, -d.z); }
	} else {
		ma = a.z;
		if (d.z > 0.0) { face = 4.0; uv = vec2(d.x, -d.y); } else { face = 5.0; uv = vec2(-d.x, -d.y); }
	}
	uv = (uv / ma) * 0.5 + 0.5;
	float h = float(textureSize(s, 0).y) / 6.0;
	h = max(h * exp2(-ceil(max(lod, 0.0))), 1.0);
	uv.y = clamp(uv.y, 0.5 / h, 1.0 - 0.5 / h);
	return vec2(uv.x, (uv.y + face) / 6.0);
}
vec4 pvr_cube(sampler2D s, vec3 d) { return texture(s, pvr_cubeUV(s, d)); }
vec4 pvr_cubeLod(sampler2D s, vec3 d, float lod) { return textureLod(s, pvr_cubeUVLod(s, d, lod), lod); }
ivec2 pvr_cubeSize(sampler2D s, int lod) { ivec2 sz = textureSize(s, lod); return ivec2(sz.x, sz.y / 6); }
vec2 pvr_volUV(sampler2D s, float layers, vec2 uv, float k) {
	float cols = ceil(sqrt(layers - 0.5));
	float rows = ceil(layers / cols);
	vec2 sz = vec2(textureSize(s, 0)) / vec2(cols, rows);
	vec2 st = clamp(uv, 0.5 / sz, 1.0 - 0.5 / sz);
	return (vec2(mod(k, cols), floor(k / cols)) + st) / vec2(cols, rows);
}
vec4 pvr_vol(sampler2D s, float layers, vec3 c) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(texture(s, pvr_volUV(s, layers, c.xy, z0)), texture(s, pvr_volUV(s, layers, c.xy, z1)), z - z0);
}
vec4 pvr_volLod(sampler2D s, float layers, vec3 c, float lod) {
	float z = clamp(c.z * layers - 0.5, 0.0, layers - 1.0);
	float z0 = floor(z);
	float z1 = min(z0 + 1.0, layers - 1.0);
	return mix(textureLod(s, pvr_volUV(s, layers, c.xy, z0), 0.0), textureLod(s, pvr_volUV(s, layers, c.xy, z1), 0.0), z - z0);
}
vec4 pvr_volFetch(sampler2D s, float layers, ivec3 c, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return texelFetch(s, c.xy + ivec2(c.z % cols, c.z / cols) * sz, 0);
}
ivec3 pvr_volSize(sampler2D s, float layers, int lod) {
	int cols = int(ceil(sqrt(layers - 0.5)));
	int rows = (int(layers) + cols - 1) / cols;
	ivec2 sz = textureSize(s, 0) / ivec2(cols, rows);
	return ivec3(sz, int(layers));
}
void main() {
  gl_FragColor = pvr_vol(tex, pvr_layers_tex, v) + texture2D(tex, v.xy);
}"#
    );
    let e: Vec<&str> = vec![r#"sampler tex -> 0x8b5f"#];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            1,
            vec![
                r#"pvr_layers_tex off=0 dwords=1 type=0x1406 len=1 binding=0 inner=1"#,
                r#"tex off=65535 dwords=1 type=0x8b5f len=1 binding=0 inner=1"#
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"v"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn es300_vertex() {
    // ARB_shader_draw_parameters guard, explicit and implicit input locations, flat int output
    let src = r#"#version 300 es
#ifdef GL_ARB_shader_draw_parameters
#define BASE gl_BaseInstanceARB
#else
uniform int base;
#endif
layout(location = 3) in vec4 pos;
in vec2 uv;
flat out int id;
void main() { id = gl_InstanceID + gl_VertexID; gl_Position = pos; }
"#;
    assert_eq!(es_version(src).unwrap(), Some(300));
    let (text, n) = translate(src, r#"vs"#, 300).unwrap();
    assert_eq!(
        text,
        r#"#version 450
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
#define gl_VertexID gl_VertexIndex
#define gl_InstanceID gl_InstanceIndex

#if 0

#define BASE gl_BaseInstanceARB

#else
layout(push_constant) uniform pvr_U {
	int base;
};

#endif
layout(location = 3) in vec4 pos;
layout(location = 4) in vec2 uv;
layout(location = 0) flat out int id;

void main() { id = gl_InstanceID + gl_VertexID; gl_Position = pos; }"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            1,
            vec![r#"base off=0 dwords=1 type=0x1404 len=1 binding=0 inner=1"#]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"id"#.to_string())];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![0];
    assert_eq!(f, e);
    let mut a: Vec<_> = reflect_attribs(&text).unwrap().into_iter().collect();
    a.sort();
    let e: Vec<(i64, (String, i64))> = vec![(3, (r#"pos"#.to_string(), 0x8b52)), (4, (r#"uv"#.to_string(), 0x8b50))];
    assert_eq!(a, e);
}

#[test]
fn python_whitespace() {
    // U+001C is whitespace to Python's \s
    let src = "uniform\u{1c}vec4 col;\nvoid main() { gl_FragColor = col; }\n";
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
layout(push_constant) uniform pvr_U {
	vec4 col;
};

void main() { gl_FragColor = col; }"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            4,
            vec![r#"col off=0 dwords=4 type=0x8b52 len=1 binding=0 inner=1"#]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn crlf_source() {
    // a CRLF source is translated as bytes (no newline translation)
    let src = "attribute vec4 pos;\r\nuniform float s;\r\nvoid main() {\r\n  gl_Position = pos * s;\r\n}\r\n";
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"vs"#, 100).unwrap();
    assert_eq!(text, "#version 450\n#define texture2D texture\n#define texture2DProj textureProj\n#define texture2DLod textureLod\n#define texture2DProjLod textureProjLod\nlayout(location = 0) in vec4 pos;\nlayout(push_constant) uniform pvr_U {\n\tfloat s;\n};\n\r\nvoid main() {\r\n  gl_Position = pos * s;\r\n}");
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            1,
            vec![r#"s off=0 dwords=1 type=0x1406 len=1 binding=0 inner=1"#]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"vs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
    let mut a: Vec<_> = reflect_attribs(&text).unwrap().into_iter().collect();
    a.sort();
    let e: Vec<(i64, (String, i64))> = vec![(0, (r#"pos"#.to_string(), 0x8b52))];
    assert_eq!(a, e);
}

#[test]
fn defines_across_lines_and_unicode_digits() {
    // `#define N\n 3` is a define (\s spans lines) and one inside a comment still counts, with an Arabic-Indic digit
    let src = r#"#define N
 3
uniform float a[N];
/*
#define M ٣
*/
uniform float b[M];
void main() { gl_FragColor = vec4(a[0] + b[0]); }
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod

#define N
 3
uniform float a[N];layout(push_constant) uniform pvr_U {
	float b[3];
};

void main() { gl_FragColor = vec4(a[0] + b[0]); }"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    /* The reflection scans the comment-stripped translated text, where the
     * `#define M` -- which lived in a comment -- is gone. The translation
     * used to emit `float b[M];`, so the reflection could not read the size
     * back and the uniform was lost; worse, the text it emitted would not
     * have compiled on its own. The size is re-emitted as the number it
     * evaluates to, so both work. */
    assert_eq!(
        reflect_uniforms(&text, &n).unwrap().0.iter().map(|u| (u.name.as_str(), u.array_len)).collect::<Vec<_>>(),
        vec![("b", 3)]
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn preprocessor_inside_a_line() {
    // a '#' after code on the same line; #extension and #version dropped
    let src = r#"uniform vec4 a; #define X 1
void main() { gl_FragColor = a; }
#extension GL_OES_standard_derivatives : enable
  #  version 100
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
layout(push_constant) uniform pvr_U {
	vec4 a;
};

#define X 1
void main() { gl_FragColor = a; }"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(
        uniforms(&text, &n),
        (
            4,
            vec![r#"a off=0 dwords=4 type=0x8b52 len=1 binding=0 inner=1"#]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn stray_closing_brace() {
    // negative depth never splits
    let src = r#"} uniform vec4 a;
void main() { gl_FragColor = a; }
"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
} uniform vec4 a;
void main() { gl_FragColor = a; }
"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(uniforms(&text, &n), (0, Vec::<String>::new()));
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn trailing_statement() {
    // a final statement without ';'
    let src = r#"void main() { gl_FragColor = vec4(1.0); }
uniform vec4 tail"#;
    assert_eq!(es_version(src).unwrap(), Some(100));
    let (text, n) = translate(src, r#"fs"#, 100).unwrap();
    assert_eq!(
        text,
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
void main() { gl_FragColor = vec4(1.0); }
uniform vec4 tail"#
    );
    let e: Vec<&str> = vec![];
    assert_eq!(names(&n), e);
    assert_eq!(uniforms(&text, &n), (0, Vec::<String>::new()));
    let mut v: Vec<_> = reflect_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![];
    assert_eq!(v, e);
    let mut f: Vec<_> = reflect_flat_varyings(&text, r#"fs"#).unwrap().into_iter().collect();
    f.sort();
    let e: Vec<i64> = vec![];
    assert_eq!(f, e);
}

#[test]
fn unroll_loops_backreference() {
    // the loop header needs a backreference (`\1`), matched by hand: `forfor` still matches at
    // its second `for`, `float(j)` guards, `(j) * 0 + (1)` indexing and a loop without arrays
    let text = r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
precision highp float;
#define MAX 3
layout(push_constant) uniform pvr_U {
	vec4 arr[MAX];
	vec4 two[4];
	int count;
};

void main() {
  vec4 c = vec4(0.0);
  for (int i = 0; i < count; i++) { c += arr[i]; if (true) { c += two[ i ]; } }
  for (int j=0;float(j)<float(count);j++) { c += arr[(j) * 2 + (1)]; }
  for (int k = 0; k < count; k++) { c += vec4(float(k)); }
  forfor (int m = 0; m < count; m++) { c += two[m]; }
  gl_FragColor = c;
}"#;
    let sizes: HashMap<String, i64> = [("arr".to_string(), 3), ("two".to_string(), 4)].into_iter().collect();
    assert_eq!(
        unroll_uniform_loops(text, &sizes).unwrap(),
        r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
precision highp float;
#define MAX 3
layout(push_constant) uniform pvr_U {
	vec4 arr[MAX];
	vec4 two[4];
	int count;
};

void main() {
  vec4 c = vec4(0.0);
  /* unrolled i x3 */ {
if (0 < count) { c += arr[0]; if (true) { c += two[ 0 ]; } }
if (1 < count) { c += arr[1]; if (true) { c += two[ 1 ]; } }
if (2 < count) { c += arr[2]; if (true) { c += two[ 2 ]; } }
}
  /* unrolled j x1 */ {
if (float(0) < float(count)) { c += arr[(0) * 2 + (1)]; }
}
  for (int k = 0; k < count; k++) { c += vec4(float(k)); }
  for/* unrolled m x4 */ {
if (0 < count) { c += two[0]; }
if (1 < count) { c += two[1]; }
if (2 < count) { c += two[2]; }
if (3 < count) { c += two[3]; }
}
  gl_FragColor = c;
}"#
    );
    let div0 = "for (int i = 0; i < n; i++) { c += arr[(i)*0+(1)]; }";
    assert!(matches!(unroll_uniform_loops(div0, &sizes), Err(Error::Fatal(_))));
    // an unterminated body: Python's slice drops the last character
    assert_eq!(
        unroll_uniform_loops(r#"for (int i = 0; i < n; i++) { c += arr[i];"#, &sizes).unwrap(),
        r#"/* unrolled i x3 */ {
if (0 < n) { c += arr[0]}
if (1 < n) { c += arr[1]}
if (2 < n) { c += arr[2]}
}"#
    );
}

#[test]
fn a_matrix_attribute_becomes_one_input_per_column() {
    /* pco_shc cannot lower a matrix input (`pco_nir_pvfio.c:909:
     * lower_pvi: Assertion 'attrib' failed`) and struct shc_attrib holds one
     * location with one to four components, so a matN attribute is written
     * as N column inputs at the N consecutive locations GL already says it
     * occupies, and reassembled at the top of main(). The matrix variable is
     * declared at file scope -- an initialiser there would not be a constant
     * expression, and a local in main() would be invisible to a function
     * declared above it. */
    let src =
        "#version 100\nattribute mat3 m;\nattribute vec4 pos;\nvoid main() { gl_Position = vec4(m[0], 1.0) + pos; }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    for c in 0..3 {
        assert!(text.contains(&format!("layout(location = {c}) in vec3 pvr_mat_m_col{c};")), "{text}");
    }
    assert!(text.contains("\nmat3 m;\n"), "the matrix is a file-scope variable: {text}");
    assert!(text.contains("m = mat3(pvr_mat_m_col0, pvr_mat_m_col1, pvr_mat_m_col2);"), "{text}");
    assert!(!text.contains("in mat3"), "pco_shc must never see a matrix input: {text}");
    /* The next attribute starts after the three columns. */
    assert!(text.contains("layout(location = 3) in vec4 pos;"), "{text}");
    /* And the reflection still calls it `m`, a GL_FLOAT_MAT3, at each of the
     * locations it occupies: glGetAttribLocation of a matrix attribute is the
     * first of them, and the application binds each column itself. */
    let mut a: Vec<(i64, (String, i64))> = reflect_attribs(&text).unwrap().into_iter().collect();
    a.sort();
    assert_eq!(
        a,
        vec![
            (0, ("m".to_string(), 0x8b5b)),
            (1, ("m".to_string(), 0x8b5b)),
            (2, ("m".to_string(), 0x8b5b)),
            (3, ("pos".to_string(), 0x8b52)),
        ]
    );
}

#[test]
fn a_matrix_attribute_is_assembled_after_the_hoisted_position() {
    /* pco_shc hands out vtxin registers in order of first use and the
     * position has to be vtxin0, which is what hoist_gl_position is for; the
     * assembly must not read a column before it. */
    let src = "#version 100\nattribute vec4 dEQP_Position;\nattribute mat2 in0;\nvarying mat2 out0;\nvoid main() {\n  out0 = in0;\n  gl_Position = dEQP_Position;\n}\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    let pos = text.find("gl_Position = dEQP_Position;").expect("hoisted");
    let asm = text.find("in0 = mat2(").expect("assembled");
    assert!(pos < asm, "the position is read first:\n{text}");
}

#[test]
fn a_matrix_attribute_the_position_itself_reads_is_assembled_first() {
    let src = "#version 100\nattribute mat4 xf;\nattribute vec4 p;\nvoid main() { gl_Position = xf * p; }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    let asm = text.find("xf = mat4(").expect("assembled");
    let pos = text.find("gl_Position = xf * p;").expect("position");
    assert!(asm < pos, "it has to exist before the statement that reads it:\n{text}");
}

#[test]
fn translation_errors() {
    assert_eq!(
        exit_msg(translate(
            r#"struct L { vec4 c[2]; };
struct B { L l[2]; };
uniform B b;
void main() { gl_FragColor = b.l[0].c; }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: whole inner array 'b_l_c' of an array-of-structs member referenced"#
    );
    /* texelFetch() on a cube map: the stacked emulation refused it by name;
     * since native cube step C the cube is a real samplerCube and the call
     * passes through unchanged, for glslang to refuse (GLSL has no
     * texelFetch on a samplerCube). */
    let (text, _) = translate(
        r#"#version 300 es
precision highp float;
uniform samplerCube env;
out vec4 o;
void main() { o = texelFetch(env, ivec3(0), 0); }
"#,
        r#"fs"#,
        300,
    )
    .expect("passed through to glslang");
    assert!(text.contains("uniform samplerCube env;") && text.contains("texelFetch(env, ivec3(0), 0)"), "{text}");
    assert_eq!(
        exit_msg(translate(
            r#"uniform vec4 a b;
void main() { gl_FragColor = a; }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: cannot parse GLSL ES declarator 'a b'"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"attribute sampler2D s;
void main() { gl_Position = vec4(0.0); }
"#,
            r#"vs"#,
            100
        )),
        r#"glsl2usc: attribute type 'sampler2D' is not supported"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"uniform float3 x;
void main() { gl_FragColor = vec4(0.0); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: uniform type 'float3' is not supported (structs are not)"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"uniform float x[NOPE];
void main() { gl_FragColor = vec4(0.0); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: array size 'NOPE' is not an integer literal or a #define'd integer"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"struct S { float a; };
varying S s;
void main() { gl_FragColor = vec4(s.a); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: varying type 'S' is not supported"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"struct Inner { float a; };
struct S { Inner i[2]; };
uniform S s[2];
void main() { gl_FragColor = vec4(s[0].i[0].a); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: struct inside an array of structs ('s[].i') is not supported"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"struct S { sampler2D t; };
uniform S s;
void main() { gl_FragColor = texture2D(s.t, vec2(0.0)); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: uniform struct member 's.t' has type 'sampler2D' (samplers in structs are not supported)"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"struct I { float a; };
struct S { I i; };
struct B { S s[2]; };
uniform B b;
void main() { gl_FragColor = vec4(b.s[0].i.a); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: struct inside an array of structs ('b.s[].i') is not supported"#
    );
    assert_eq!(
        exit_msg(translate(
            r#"struct S { sampler2D t; };
struct B { S s[2]; };
uniform B b;
void main() { gl_FragColor = vec4(0.0); }
"#,
            r#"fs"#,
            100
        )),
        r#"glsl2usc: array-of-structs member 'b.s[].t' has type 'sampler2D'"#
    );
    assert!(matches!(
        translate(
            r#"struct S { float a; S s; };
uniform S u;
void main() { gl_FragColor = vec4(u.a); }
"#,
            "fs",
            100
        ),
        Err(Error::Fatal(_))
    ));
    // non-ASCII code outside comments is refused (glslang rejects it too)
    assert!(exit_msg(translate("uniform float \u{e9};", "fs", 100)).contains("non-ASCII"));
}

#[test]
fn reflection_errors_and_450_sources() {
    assert_eq!(
        exit_msg(reflect_uniforms(
            r#"#version 450
layout(push_constant) uniform PC { vec4 a; mat3 m; float f[3]; ivec3 v; } pc;
layout(set = 0, binding = 2) uniform sampler2D t;
layout(location = 0) out vec4 o;
void main() { o = pc.a; }
"#,
            &GlNames::default()
        )),
        r#"glsl2usc: push-constant block with an instance name ('pc') is not supported: members must be global"#
    );
    assert_eq!(
        exit_msg(reflect_uniforms(
            r#"#version 450
layout(location = 0) out vec4 pvr_FragColor;
#define gl_FragColor pvr_FragColor
#define texture2D texture
#define texture2DProj textureProj
#define texture2DLod textureLod
#define texture2DProjLod textureProjLod
layout(push_constant) uniform pvr_U {
	float nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn;
};

void main() { gl_FragColor = vec4(0.0); }"#,
            &GlNames::default()
        )),
        r#"glsl2usc: uniform name 'nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn' is longer than SHC_UNIFORM_NAME_MAX-1 (63)"#
    );
    let src = r#"#version 450
#define K (4)
layout(push_constant) uniform PC { vec3 a; mat2 m[2]; float f[K]; uvec3 v; bool b; };
layout(binding = 1, set = 0) uniform sampler2DArray t;
layout(location = 0) flat in int fi;
layout(location = 2) in vec2 uv[2];
layout(location = 0) out vec4 o;
void main() { o = vec4(a, 1.0); }
"#;
    assert_eq!(es_version(src).unwrap(), None);
    assert_eq!(
        uniforms(src, &GlNames::default()),
        (
            20,
            vec![
                r#"a off=0 dwords=3 type=0x8b51 len=1 binding=0 inner=1"#,
                r#"m off=4 dwords=8 type=0x8b5a len=2 binding=0 inner=1"#,
                r#"f off=12 dwords=4 type=0x1406 len=4 binding=0 inner=1"#,
                r#"v off=16 dwords=3 type=0x8dc7 len=1 binding=0 inner=1"#,
                r#"b off=19 dwords=1 type=0x8b56 len=1 binding=0 inner=1"#,
                r#"t off=65535 dwords=1 type=0x8dc1 len=1 binding=1 inner=1"#
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        )
    );
    let mut v: Vec<_> = reflect_varyings(src, "fs").unwrap().into_iter().collect();
    v.sort();
    let e: Vec<(i64, String)> = vec![(0, r#"fi"#.to_string()), (2, r#"uv"#.to_string())];
    assert_eq!(v, e);
    let f: Vec<_> = reflect_flat_varyings(src, "fs").unwrap().into_iter().collect();
    let e: Vec<i64> = vec![0];
    assert_eq!(f, e);
    assert_eq!(
        es_version(
            r#"#version 300
varying vec4 x;"#
        )
        .unwrap(),
        None
    );
    assert_eq!(
        es_version(
            r#"  # version 310 essential
"#
        )
        .unwrap(),
        Some(310)
    );
    /* No #version directive is GLSL ES 1.00, whatever the text says: the ES
     * specification defines the default, and a version-less shader that
     * happens not to mention attribute/varying/gl_FragColor is still one. */
    assert_eq!(es_version(r#"void main(){}"#).unwrap(), Some(100));
    assert_eq!(es_version(r#"void f(){ gl_FragData[0]; }"#).unwrap(), Some(100));
    assert_eq!(es_version(r#"/* #version 300 es */ attribute vec4 a;"#).unwrap(), Some(100));
    assert_eq!(
        es_version(
            r#"#version 300 es
"#
        )
        .unwrap(),
        Some(300)
    );
    assert_eq!(
        es_version(
            r#"#version 100
"#
        )
        .unwrap(),
        Some(100)
    );
    assert_eq!(
        es_version(
            r#"#version 0100
"#
        )
        .unwrap(),
        None
    );
}

#[test]
fn a_local_that_shadows_a_parameter_gets_a_scope_of_its_own() {
    /* GLSL ES 1.00 lets a local shadow a parameter; GLSL 4.50 calls the
     * second declaration a redefinition, so the rewrite turned a legal ES
     * shader into an error (dEQP's
     * shaders.scoping.valid.local_variable_hides_function_parameter). One
     * more pair of braces restores the two scopes. */
    let src = "#version 100\nint func (int inp, int x) { int x = 5; return inp + x - 5; }\nvoid main() { gl_Position = vec4(float(func(1, 42))); }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    assert!(text.contains("int x) {{ int x = 5;"), "the body is nested: {text}");
}

#[test]
fn a_function_that_shadows_nothing_is_left_alone() {
    /* The rewrite must touch only the shaders that need it: every other
     * one keeps its text, and with it its row of the offline table. */
    let src = "#version 100\nint func (int inp) { int y = 5; return inp + y; }\nvoid main() { gl_Position = vec4(float(func(1))); }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    assert!(text.contains("int func (int inp) { int y = 5;"), "unchanged: {text}");
    assert!(!text.contains("{{"), "no extra scope: {text}");
}

#[test]
fn a_parameter_merely_read_is_not_a_shadow() {
    /* `return inp + x;` mentions both parameters and declares neither. A
     * rule that looked for the name alone would nest every function. */
    let src = "#version 100\nint func (int inp, int x) { return inp + x; }\nvoid main() { gl_Position = vec4(float(func(1, 2))); }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    assert!(!text.contains("{{"), "no extra scope: {text}");
}

#[test]
fn main_is_never_nested() {
    /* main() takes no parameters, so it cannot shadow one -- which matters
     * because hoist_gl_position and the matrix assembly both insert at the
     * top of its body. */
    let src = "#version 100\nattribute vec4 p;\nvoid main() { vec4 p2 = p; gl_Position = p2; }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    assert!(!text.contains("{{"), "{text}");
}

#[test]
fn a_gles_100_cube_fetch_is_a_native_cube_fetch() {
    /* GLSL ES 1.00 spells it textureCube(); the emulation rewrites the call
     * text itself, so the call has to carry the 450 name before the
     * emulation runs. And in GLSL 450 `textureCube` is an opaque TYPE, so an
     * un-rewritten call reads as a constructor: glslang answers
     * "'sampler/image' : cannot construct this type", which says nothing
     * about the real problem. dEQP's shaders.texture_functions.*.texturecube
     * is four cases of it. Since native cube step C the cube is the TPU's
     * own: the 450 fetch on a real samplerCube, no stacked emulation. */
    let src = "uniform lowp samplerCube u_s;\nvarying mediump vec3 v_c;\nvoid main() { gl_FragColor = textureCube(u_s, v_c); }\n";
    let (text, _) = translate(src, "fs", 100).unwrap();
    assert!(!text.contains("textureCube("), "the ES name is gone: {text}");
    assert!(text.contains("texture(u_s, v_c)"), "the 450 fetch: {text}");
    assert!(text.contains("uniform samplerCube u_s;"), "a real cube, not a stacked 2D texture: {text}");
    assert!(!text.contains("pvr_cube"), "no emulation helper: {text}");
}

#[test]
fn the_cube_lod_fetch_is_rewritten_before_the_shorter_name() {
    /* textureCubeLod has textureCube as a prefix, so the longer name has to
     * be rewritten first or it becomes `texture`+`Lod(`. */
    let src = "uniform lowp samplerCube u_s;\nattribute vec3 a_c;\nvoid main() { gl_Position = textureCubeLod(u_s, a_c, 0.0); }\n";
    let (text, _) = translate(src, "vs", 100).unwrap();
    assert!(!text.contains("textureCube"), "{text}");
    assert!(text.contains("textureLod(u_s, a_c, 0.0)"), "{text}");
}

#[test]
fn a_matrix_is_columns_of_rows_and_both_answers_come_from_one_place() {
    /* matCxR is C columns of R rows -- the opposite order from what the name
     * reads like -- so a mat4x2 is four vec2s, not two vec4s. es100_loc and
     * column_type must agree about that or a non-square matrix attribute
     * gets the wrong number of locations or the wrong column type. */
    for (typ, columns, col) in [
        ("mat2", 2, "vec2"),
        ("mat3", 3, "vec3"),
        ("mat4", 4, "vec4"),
        ("mat2x4", 2, "vec4"),
        ("mat4x2", 4, "vec2"),
        ("mat3x2", 3, "vec2"),
        ("mat2x3", 2, "vec3"),
    ] {
        assert_eq!(super::matrix_shape(typ), Some((columns, col)), "{typ}");
        assert_eq!(super::es100_loc(typ), Some(columns), "{typ}");
        assert_eq!(super::column_type(typ), col, "{typ}");
    }
    for typ in ["vec4", "mat", "mat1", "mat5", "mat2x5", "matrix", "sampler2D"] {
        assert_eq!(super::matrix_shape(typ), None, "{typ}");
    }
    assert_eq!(super::es100_loc("vec4"), Some(1));
    assert_eq!(super::es100_loc("sampler2D"), None);
}

#[test]
fn an_array_size_is_an_integer_constant_expression() {
    /* GLSL ES 1.00 section 4.3.3. The translation runs after the
     * preprocessor now, so a macro arrives already expanded and what is left
     * is arithmetic: dEQP writes `u_lights[${NUM_DIR}+${NUM_OMNI}]`, which
     * preprocesses to `u_lights[2+1]`. */
    let d = std::collections::HashMap::from([("N".to_string(), 4i64)]);
    for (expr, want) in [
        ("3", 3),
        (" 2 + 1 ", 3),
        ("2*3", 6),
        ("2*3+1", 7),
        ("1+2*3", 7),
        ("(1+2)*3", 9),
        ("8/2", 4),
        ("7%4", 3),
        ("-(-2)", 2),
        ("+3", 3),
        ("10u", 10),
        ("N", 4),
        ("N+1", 5),
        ("N*N", 16),
    ] {
        assert_eq!(super::resolve_size(expr, &d).unwrap(), want, "{expr}");
    }
    for bad in ["0", "-1", "2-2", "1/0", "(1", "1)", "", "M", "1.5", "1 2"] {
        assert!(super::resolve_size(bad, &d).is_err(), "{bad} should not be an array size");
    }
}

#[test]
fn an_array_of_struct_uniforms_is_flattened_per_member() {
    /* dEQP's light shader: `uniform Light u_lights[N]`. One flattened array
     * per member; `u_lights[i].member` becomes `u_lights_member[i]`, and the
     * reflection reports the GL name with the array length, which is what
     * glGetActiveUniform must enumerate. */
    let src = "struct Light { highp vec3 color; highp float atten[3]; };\nuniform Light u_lights[2];\nattribute vec4 a_p;\nvoid main() { gl_Position = a_p * u_lights[1].atten[2] + vec4(u_lights[0].color, 1.0); }\n";
    let (text, n) = translate(src, "vs", 100).unwrap();
    assert!(text.contains("\tvec3 u_lights_color[2];"), "{text}");
    assert!(text.contains("\tfloat u_lights_atten[6];"), "one array of N*M: {text}");
    assert!(text.contains("u_lights_atten[(1)*3+(2)]"), "the two indices fold into one: {text}");
    assert!(text.contains("u_lights_color[0]"), "{text}");
    let (u, _) = reflect_uniforms(&text, &n).unwrap();
    let got: Vec<(&str, i64, i64)> = u.iter().map(|x| (x.name.as_str(), x.array_len, x.inner)).collect();
    assert_eq!(got, vec![("u_lights[0].color", 2, 1), ("u_lights[0].atten", 6, 3)]);
}

#[test]
fn a_struct_inside_an_array_of_struct_uniforms_is_refused_with_its_path() {
    let src =
        "struct I { float a; };\nstruct S { I i; };\nuniform S s[2];\nvoid main() { gl_FragColor = vec4(s[0].i.a); }\n";
    let m = exit_msg(translate(src, "fs", 100));
    assert!(m.contains("struct inside an array of structs ('s[].i')"), "{m}");
}
