// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools shaderc-compile`: compile one shader for the remote shader
//! back end, as JSON in and JSON out.
//!
//! The offline back end (`src/shaderc/offline.rs`) can only run shaders that
//! were compiled into `blobs.rs` at build time. The Khronos dEQP-GLES2 suite
//! writes its shaders as it runs, so there is nothing to precompile, and the
//! PCO toolchain does not exist on the target. `src/shaderc/remote.rs` sends
//! the GLSL text to `tools/shaderc_server.py` on a host instead, and that
//! server runs this command once per shader.
//!
//! The compile itself is [`glsl2usc::resolve_shader`] and the reply is built
//! from [`emit::build`] — the very functions that produce `blobs.rs`. Nothing
//! about the pipeline is duplicated: a shader compiled here and the same
//! shader in the offline table are identical dword for dword and field for
//! field, which `tests/shaderc_remote.rs` checks for every table row.
//!
//! One request object on stdin, one reply object on stdout:
//!
//! ```text
//!   compile  {"op":"compile","stage":"vert"|"frag","source":"<GLSL text>",
//!             "blend_key":<uint32>}          # or "blend":"<spec>","colormask":"rgba"
//!         -> {"ok":true,"blob":"<base64 USC dwords, little-endian>",
//!             "usc_dw":<n>,"origin":"pco_shc","reflection":{...}}
//!         -> {"ok":false,"error":"<compiler log>"}
//!
//!   link     {"op":"link","vert":"<VS GLSL>","frag":"<FS GLSL>",
//!             "max_varying_vectors":<n>}     # optional: the client GPU's limit
//!         -> {"ok":true}   |   {"ok":false,"error":"<ES link log>"}
//! ```
//!
//! The reflection keys are the field names of `ShcShader` (`src/shaderc/mod.rs`).
//! A field the stage does not fill is 0, exactly as the generated table has it.
//!
//! The exit status is 0 whenever a reply was produced, a refused shader
//! included: the server forwards the object either way. A non-zero status is a
//! malformed request or a broken environment.

use crate::emit;
use crate::glsl2usc;
use crate::link as eslink;
use crate::py::{self, re};
use crate::{Error, R};
use serde_json::{json, Map, Value};
use std::io::Read;
use yakogl::shaderc::ShcShader;

const USAGE: &str = "usage: yakogl-tools shaderc-compile [-h]

Reads one JSON request on stdin and writes one JSON reply on stdout; see
tools/shaderc_server.py, which is the only caller.";

/* ---- the GLSL text as a file the pipeline can open ------------------------ */

/// `resolve_shader()` takes a path (it looks for `.prebuilt.json` /
/// `.override` sidecars beside it and names the file in compiler messages), so
/// the request's source text is written to a private temp directory first. The
/// basename is what the compiler logs, so keep it plain.
struct SourceFile {
    /// Removed when this value drops, which is after the compile has run.
    _dir: tempfile::TempDir,
    path: String,
}

impl SourceFile {
    fn new(source: &str, ext: &str) -> R<SourceFile> {
        let dir = tempfile::Builder::new()
            .prefix("shaderc-compile.")
            .rand_bytes(8)
            .tempdir()
            .map_err(|e| Error::Fatal(format!("mkdtemp: {e}")))?;
        let path = py::join(&dir.path().to_string_lossy(), &format!("shader{ext}"));
        py::write_text(&path, source)?;
        Ok(SourceFile { _dir: dir, path })
    }
}

/// The compiler names the shader by its temp path, and the application reads
/// that text through `glGetShaderInfoLog`. Strip the directory so the log
/// says `shader.frag:3: ...` and not a path nobody can act on.
///
/// Two directories appear: this command's, holding the source as it arrived,
/// and `run_pco()`'s, holding the GLSL 450 rewrite glslang actually saw. Both
/// are named `<tool>.<8 random chars>`, so one pattern covers them.
fn scrub_paths(text: &str) -> String {
    py::sub_lit(&re(r"(?:/[\w.-]+)*/(?:glsl2usc|shaderc-compile)\.\w+/"), text, "")
}

/* ---- base64 (the USC binary travels as one JSON string) ------------------- */

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let (b0, b1, b2) = (c[0] as u32, *c.get(1).unwrap_or(&0) as u32, *c.get(2).unwrap_or(&0) as u32);
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/* ---- struct shc_shader -> the reflection object --------------------------- */

fn name_of(field: &[core::ffi::c_char]) -> String {
    String::from_utf8_lossy(yakogl::util::name_bytes(field)).into_owned()
}

/// Every field of [`ShcShader`] the offline table stores, under the field's
/// own name. `usc`/`usc_dw` travel separately (`blob`), and `priv_` is the
/// back end's, not the compiler's.
fn reflection_of(sh: &ShcShader) -> Value {
    let varyings: Vec<Value> = sh.varyings[..sh.num_varyings as usize]
        .iter()
        .map(|v| {
            json!({"location": v.location, "components": v.components,
                   "interp": v.interp, "vtxout_off": v.vtxout_off, "name": name_of(&v.name)})
        })
        .collect();
    let attribs: Vec<Value> = sh.attribs[..sh.num_attribs as usize]
        .iter()
        .map(|a| {
            json!({"location": a.location, "components": a.components, "vi_reg": a.vi_reg,
                   "format": a.format, "name": name_of(&a.name), "gl_type": a.gl_type})
        })
        .collect();
    let uniforms: Vec<Value> = sh
        .uniform_table()
        .iter()
        .map(|u| {
            json!({"name": name_of(&u.name), "dword_off": u.dword_off, "dwords": u.dwords,
                   "gl_type": u.gl_type, "array_len": u.array_len, "binding": u.binding, "inner": u.inner})
        })
        .collect();
    json!({
        "temps": sh.temps,
        "num_varyings": sh.num_varyings,
        "varyings": varyings,
        "uses_w": sh.uses_w,
        "uses_discard": sh.uses_discard,
        "reads_front_facing": sh.reads_front_facing,
        "meta_off": sh.meta_off,
        "fs_front_face_off1": sh.fs_front_face_off1,
        "num_outputs": sh.num_outputs,
        "num_tile_buffers": sh.num_tile_buffers,
        "tile_buffer_sh": sh.tile_buffer_sh,
        "num_attribs": sh.num_attribs,
        "attribs": attribs,
        "vs_output_size": sh.vs_output_size,
        "vs_psize_off": sh.vs_psize_off,
        "vs_vertex_id_reg1": sh.vs_vertex_id_reg1,
        "vs_instance_id_reg1": sh.vs_instance_id_reg1,
        "uniform_dwords": sh.uniform_dwords,
        "uniform_sh_dwords": sh.uniform_sh_dwords,
        "num_uniforms": sh.num_uniforms,
        "uniforms": uniforms,
        "blend_key": sh.blend_key,
        "uses_fbfetch": sh.uses_fbfetch,
        "blend_consts_off": sh.blend_consts_off,
        "uses_pntc": sh.uses_pntc,
        "uses_z": sh.uses_z,
        "fragcoord_ytf_off1": sh.fragcoord_ytf_off1,
        "native_cube": sh.native_cube,
    })
}

/* ---- the requests --------------------------------------------------------- */

fn req_str(req: &Map<String, Value>, key: &str) -> Option<String> {
    match req.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// `"vert"`/`"vs"`/`"vertex"` and `"frag"`/`"fs"`/`"fragment"` -> the file
/// extension the pipeline reads the stage from.
fn stage_ext(stage: &str) -> Option<&'static str> {
    match stage {
        "vert" | "vs" | "vertex" => Some(".vert"),
        "frag" | "fs" | "fragment" => Some(".frag"),
        _ => None,
    }
}

/// The `--blend` spec this request asks for: either given as text, or rebuilt
/// from the key the GL core computed from the draw's blend state.
fn blend_spec(req: &Map<String, Value>) -> R<String> {
    if let Some(mut spec) = req_str(req, "blend") {
        if let Some(mask) = req_str(req, "colormask") {
            spec += &format!(":{mask}");
        }
        return Ok(spec);
    }
    let key = match req.get("blend_key") {
        Some(v) => py::py_int_json(v, false)?,
        None => 0,
    };
    let key = u32::try_from(key).map_err(|_| Error::Exit(format!("blend_key {key} is not a uint32")))?;
    glsl2usc::spec_from_key(key)
}

fn compile(req: &Map<String, Value>) -> R<Value> {
    let Some(stage) = req_str(req, "stage") else {
        return Ok(json!({"ok": false, "error": "request has no stage"}));
    };
    let Some(ext) = stage_ext(&stage) else {
        return Ok(json!({"ok": false, "error": format!("stage must be vert or frag (got {})", py::repr(&stage))}));
    };
    let Some(source) = req_str(req, "source") else {
        return Ok(json!({"ok": false, "error": "request has no source"}));
    };
    let spec = match blend_spec(req) {
        Ok(s) => s,
        /* an unknown factor code is the caller's error, not a broken host */
        Err(Error::Exit(m)) => return Ok(json!({"ok": false, "error": m})),
        Err(e) => return Err(e),
    };

    let src = SourceFile::new(&source, ext)?;
    /* Strictness::Runtime: this is an application's glCompileShader, so a
     * text the GLSL ES rules reject must fail here with the log. The offline
     * table generator is the lenient caller, and for the reason written down
     * on `Strictness` it has to stay that way. */
    let (sh, origin) = match glsl2usc::resolve_shader_mode(&src.path, &spec, glsl2usc::Strictness::Runtime) {
        /* Error::Exit is how the pipeline reports a diagnosed failure: a
         * glslang syntax error, a pco_shc refusal, a shader outside the ABI.
         * That text is what glGetShaderInfoLog hands the application. */
        Err(Error::Exit(m)) => return Ok(json!({"ok": false, "error": scrub_paths(&m)})),
        Err(e) => return Err(e),
        Ok(Err(why)) => return Ok(json!({"ok": false, "error": format!("no variant for this blend state: {why}")})),
        Ok(Ok(pair)) => pair,
    };
    let row = emit::build(&sh)?;
    let mut raw = Vec::with_capacity(row.usc().len() * 4);
    for dw in row.usc() {
        raw.extend_from_slice(&dw.to_le_bytes());
    }
    Ok(json!({
        "ok": true,
        "blob": b64_encode(&raw),
        "usc_dw": row.usc().len(),
        "origin": origin,
        "reflection": reflection_of(&row.shader),
    }))
}

fn link(req: &Map<String, Value>) -> R<Value> {
    let (Some(vert), Some(frag)) = (req_str(req, "vert"), req_str(req, "frag")) else {
        return Ok(json!({"ok": false, "error": "link needs both a vert and a frag source"}));
    };
    /* the client's GL_MAX_VARYING_VECTORS: this binary is built for no GPU */
    let rows = match req.get("max_varying_vectors") {
        None => yakogl::gl::context::MAX_VARYING_VECTORS,
        Some(v) => match v.as_u64().and_then(|n| usize::try_from(n).ok()) {
            Some(n) => n,
            None => return Ok(json!({"ok": false, "error": "max_varying_vectors is not a count"})),
        },
    };
    match eslink::check_for(&vert, &frag, rows) {
        Ok(()) => Ok(json!({"ok": true})),
        Err(Error::Exit(m)) => Ok(json!({"ok": false, "error": m})),
        Err(e) => Err(e),
    }
}

/* ---- the command ---------------------------------------------------------- */

pub fn main(argv: &[String]) -> R<()> {
    if let Some(a) = argv.first() {
        return match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                Ok(())
            }
            other => Err(Error::Usage(format!("{USAGE}\n\nunexpected argument '{other}'"))),
        };
    }
    let mut body = String::new();
    std::io::stdin().read_to_string(&mut body).map_err(|e| Error::Fatal(format!("reading the request: {e}")))?;
    let req: Value = serde_json::from_str(&body).map_err(|e| Error::Usage(format!("the request is not JSON: {e}")))?;
    let Value::Object(req) = req else {
        return Err(Error::Usage("the request must be a JSON object".into()));
    };
    let op = req_str(&req, "op").unwrap_or_else(|| "compile".into());
    let answer = match op.as_str() {
        "compile" => compile(&req),
        "link" => link(&req),
        other => Ok(json!({"ok": false, "error": format!("unknown op {}", py::repr(other))})),
    };
    /* The three-outcome contract: a verdict on the shader carries "ok", and
     * anything else -- a compiler that is not installed, one that cannot be
     * run -- is `{"error": ...}` with no "ok", which the client must not
     * cache and must not report to the application as a bad shader. Saying it
     * in JSON beats exiting with a message on stderr and leaving the server
     * to infer it from the absence of output. */
    let reply = match answer {
        Ok(v) => v,
        Err(Error::Fatal(m)) => json!({ "error": m }),
        Err(e) => return Err(e),
    };
    println!("{}", serde_json::to_string(&reply).map_err(|e| Error::Fatal(format!("encoding the reply: {e}")))?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yakogl::shaderc::SHC_STAGE_VERTEX;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn base64_matches_the_reference_vectors() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(b64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        /* the last two code points of the alphabet, and high bytes */
        assert_eq!(b64_encode(&[0xfb, 0xff, 0xbf]), "+/+/");
    }

    #[test]
    fn a_stage_is_named_as_the_client_or_the_pipeline_names_it() {
        assert_eq!(stage_ext("vert"), Some(".vert"));
        assert_eq!(stage_ext("vs"), Some(".vert"));
        assert_eq!(stage_ext("fragment"), Some(".frag"));
        assert_eq!(stage_ext("geometry"), None);
    }

    #[test]
    fn the_blend_spec_comes_from_the_key_or_the_text() {
        assert_eq!(blend_spec(&obj(json!({"blend_key": 0}))).unwrap(), "");
        let text = obj(json!({"blend": "src_alpha,inv_src_alpha", "colormask": "rgb"}));
        assert_eq!(blend_spec(&text).unwrap(), "src_alpha,inv_src_alpha:rgb");
        /* the text wins over a key, so a client can ask for a spec verbatim */
        let both = obj(json!({"blend": "one,one", "blend_key": 12345}));
        assert_eq!(blend_spec(&both).unwrap(), "one,one");
    }

    #[test]
    fn a_request_without_a_stage_or_a_source_is_refused_not_fatal() {
        assert_eq!(compile(&obj(json!({}))).unwrap()["error"], "request has no stage");
        assert_eq!(compile(&obj(json!({"stage": "x"}))).unwrap()["error"], "stage must be vert or frag (got 'x')");
        assert_eq!(compile(&obj(json!({"stage": "vert"}))).unwrap()["error"], "request has no source");
    }

    #[test]
    fn a_link_request_needs_both_stages() {
        assert_eq!(link(&obj(json!({"vert": "x"}))).unwrap()["error"], "link needs both a vert and a frag source");
    }

    #[test]
    fn the_reflection_carries_every_field_of_the_abi_struct() {
        /* if a field is added to ShcShader this test says so before a client
         * silently reads a zero for it */
        let r = reflection_of(&ShcShader::ZERO);
        let keys: Vec<&String> = r.as_object().unwrap().keys().collect();
        assert_eq!(keys.len(), 28, "{keys:?}");
        for k in [
            "temps",
            "uniform_sh_dwords",
            "num_tile_buffers",
            "reads_front_facing",
            "blend_consts_off",
            "uses_pntc",
            "uses_z",
            "fragcoord_ytf_off1",
            "native_cube",
        ] {
            assert!(r.get(k).is_some(), "the reflection has no {k}");
        }
    }

    #[test]
    fn a_compile_log_names_the_shader_and_not_the_temp_directory() {
        assert_eq!(
            scrub_paths(
                "glsl2usc: glslangValidator failed for /tmp/glsl2usc.KjEhTtV5/shader.frag:\n\
                         ERROR: /tmp/glsl2usc.KjEhTtV5/shader.frag:9: 'oops' : undeclared identifier"
            ),
            "glsl2usc: glslangValidator failed for shader.frag:\n\
                         ERROR: shader.frag:9: 'oops' : undeclared identifier"
        );
    }

    /// The stage of a compiled shader is the file extension, so the two
    /// spellings the protocol accepts must not drift from the ABI's codes.
    #[test]
    fn the_vertex_stage_code_is_the_abi_one() {
        assert_eq!(SHC_STAGE_VERTEX, 0);
    }
}
