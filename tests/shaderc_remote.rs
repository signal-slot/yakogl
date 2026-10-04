// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The remote back end reproduces the offline table, row for row.
//!
//! This is the whole claim the remote path rests on: `tools/shaderc_server.py`
//! runs the same pipeline that wrote `src/shaderc/blobs.rs`, so a shader
//! fetched at run time must be the shader the table holds — the same USC
//! dwords and the same `ShcShader` fields, blend variants included. If that
//! ever stopped being true, a shader would render differently depending on
//! which back end a build was made with, which is exactly the class of bug
//! nobody would find on silicon.
//!
//! It needs a compile server, so it is a no-op when there is none: without
//! one, `cargo test` stays green and says it skipped. To run it for real:
//!
//! ```sh
//! PCO_SHC=... tools/shaderc_server.py --cache ~/.cache/yakogl-shaderc &
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-remote \
//!     --test shaderc_remote
//! ```
//!
//! `tools/shaderc_remote_test.sh` does both of those and sweeps the whole
//! table. By default this test takes a bounded sample instead, because every
//! row is a glslang + pco_shc run and the table has ~2000 of them:
//! `YAKOGL_SHADERC_TEST_ROWS=<n>` (default 64) or `=all`.
#![cfg(feature = "shaderc-remote")]

use std::ffi::c_char;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use yakogl::shaderc::remote::SHC_REMOTE;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::{ShcBlob, ShcShader, SHC_STAGE_FRAGMENT};
use yakogl::util::name_bytes;

const DEFAULT_SERVER: &str = "127.0.0.1:7777";
const DEFAULT_ROWS: usize = 64;
const LOG_SIZE: usize = 4096;

/// `YAKOGL_SHADERC_SERVER`, as the back end reads it.
fn server_addr() -> String {
    let spec = std::env::var("YAKOGL_SHADERC_SERVER").unwrap_or_default();
    if spec.is_empty() {
        DEFAULT_SERVER.to_string()
    } else if spec.rfind(':').is_some_and(|i| i > 0) {
        spec
    } else {
        format!("{spec}:7777")
    }
}

/// Is a compile server listening? Without one this test has nothing to do.
fn server_is_up() -> bool {
    let Ok(addrs) = server_addr().to_socket_addrs() else { return false };
    addrs.into_iter().any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
}

/// Compile one source through the remote back end. `Ok` is the shader, `Err`
/// the return code and the log. `ShcShader` is the C ABI struct and has no
/// `Debug`, so the failing side is unwrapped by hand rather than with
/// `expect_err`.
fn compile(stage: u32, source: &str) -> Result<ShcShader, (i32, String)> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG_SIZE];
    let text = format!("{source}\0");
    // SAFETY: a NUL-terminated source, a valid output, and a log of the size
    // announced.
    let rc = unsafe {
        SHC_REMOTE.compile.unwrap()(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len())
    };
    let text = log_text(&log);
    if rc == 0 {
        Ok(out)
    } else {
        Err((rc, text))
    }
}

fn log_text(log: &[c_char]) -> String {
    let bytes: Vec<u8> = log.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The GLSL text a table row was built from.
fn source_of(row: &ShcBlob) -> Option<&'static str> {
    let name = row.source_name();
    let text = SHC_SOURCE_TABLE.iter().find(|s| s.name == name)?.text;
    /* the table's texts carry their own trailing NUL */
    Some(text.strip_suffix('\0').unwrap_or(text))
}

/// Everything the two shaders must agree on, as a list of what differs.
fn differences(want: &ShcShader, got: &ShcShader) -> Vec<String> {
    let mut d = Vec::new();
    let mut cmp = |what: &str, a: u64, b: u64| {
        if a != b {
            d.push(format!("{what}: table {a}, remote {b}"));
        }
    };
    cmp("usc_dw", want.usc_dw.into(), got.usc_dw.into());
    cmp("temps", want.temps.into(), got.temps.into());
    cmp("uses_w", want.uses_w.into(), got.uses_w.into());
    cmp("uses_discard", want.uses_discard.into(), got.uses_discard.into());
    cmp("reads_front_facing", want.reads_front_facing.into(), got.reads_front_facing.into());
    cmp("meta_off", want.meta_off.into(), got.meta_off.into());
    cmp("fs_front_face_off1", want.fs_front_face_off1.into(), got.fs_front_face_off1.into());
    cmp("num_outputs", want.num_outputs.into(), got.num_outputs.into());
    cmp("num_tile_buffers", want.num_tile_buffers.into(), got.num_tile_buffers.into());
    cmp("tile_buffer_sh", want.tile_buffer_sh.into(), got.tile_buffer_sh.into());
    cmp("vs_output_size", want.vs_output_size.into(), got.vs_output_size.into());
    cmp("vs_psize_off", want.vs_psize_off.into(), got.vs_psize_off.into());
    cmp("vs_vertex_id_reg1", want.vs_vertex_id_reg1.into(), got.vs_vertex_id_reg1.into());
    cmp("vs_instance_id_reg1", want.vs_instance_id_reg1.into(), got.vs_instance_id_reg1.into());
    cmp("uniform_dwords", want.uniform_dwords.into(), got.uniform_dwords.into());
    cmp("uniform_sh_dwords", want.uniform_sh_dwords.into(), got.uniform_sh_dwords.into());
    cmp("blend_key", want.blend_key.into(), got.blend_key.into());
    cmp("uses_fbfetch", want.uses_fbfetch.into(), got.uses_fbfetch.into());
    cmp("blend_consts_off", want.blend_consts_off.into(), got.blend_consts_off.into());
    cmp("num_varyings", want.num_varyings.into(), got.num_varyings.into());
    cmp("num_attribs", want.num_attribs.into(), got.num_attribs.into());
    cmp("num_uniforms", want.num_uniforms.into(), got.num_uniforms.into());

    if want.usc_words() != got.usc_words() {
        let first = want.usc_words().iter().zip(got.usc_words()).position(|(a, b)| a != b);
        d.push(match first {
            Some(i) => {
                format!("usc dword {i}: table {:#010x}, remote {:#010x}", want.usc_words()[i], got.usc_words()[i])
            }
            None => "usc: different lengths".to_string(),
        });
    }
    if d.is_empty() {
        /* the reflection arrays, now that the counts are known to agree */
        for i in 0..want.num_varyings as usize {
            let (a, b) = (&want.varyings[i], &got.varyings[i]);
            if (a.location, a.components, a.interp, a.vtxout_off) != (b.location, b.components, b.interp, b.vtxout_off)
                || name_bytes(&a.name) != name_bytes(&b.name)
            {
                d.push(format!("varying {i} differs"));
            }
        }
        for i in 0..want.num_attribs as usize {
            let (a, b) = (&want.attribs[i], &got.attribs[i]);
            if (a.location, a.components, a.vi_reg, a.format, a.gl_type)
                != (b.location, b.components, b.vi_reg, b.format, b.gl_type)
                || name_bytes(&a.name) != name_bytes(&b.name)
            {
                d.push(format!("attrib {i} differs"));
            }
        }
        for (i, (a, b)) in want.uniform_table().iter().zip(got.uniform_table()).enumerate() {
            if (a.dword_off, a.dwords, a.gl_type, a.array_len, a.binding, a.inner)
                != (b.dword_off, b.dwords, b.gl_type, b.array_len, b.binding, b.inner)
                || name_bytes(&a.name) != name_bytes(&b.name)
            {
                d.push(format!("uniform {i} ('{}') differs", String::from_utf8_lossy(name_bytes(&a.name))));
            }
        }
    }
    d
}

/* ---- the same pipeline, run here ----------------------------------------
 *
 * A row can differ from the table for two reasons: the remote path is wrong,
 * or blobs.rs was generated with a different pco_shc than this host has. They
 * look identical from the table's side, so the test settles it by running the
 * pipeline itself -- `yakogl-tools shaderc-compile`, which is exactly what the
 * server runs -- and seeing which side agrees. Only the USC binary is compared
 * that way: a transport that mangled the reflection would have shown up on
 * every row that does match the table.
 */

/// The `yakogl-tools` binary, if this checkout has one built.
fn tools_binary() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("YAKOGL_TOOLS") {
        let p = std::path::PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    [".target/release", ".target/debug", "target/release", "target/debug"]
        .iter()
        .map(|d| root.join(d).join("yakogl-tools"))
        .find(|p| p.is_file())
}

/// `text` as a JSON string literal.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The value of a `"key":<digits>` member of a flat reply.
fn json_number(reply: &str, key: &str) -> Option<usize> {
    let at = reply.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = &reply[at..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The value of a `"key":"..."` member of a flat reply (no escapes in base64).
fn json_plain_string<'a>(reply: &'a str, key: &str) -> Option<&'a str> {
    let at = reply.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let rest = &reply[at..];
    Some(&rest[..rest.find('"')?])
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The USC dwords as the wire carries them, so a locally produced reply can be
/// compared with a shader the back end decoded.
fn b64_of_usc(words: &[u32]) -> String {
    let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut out = String::with_capacity(raw.len().div_ceil(3) * 4);
    for c in raw.chunks(3) {
        let n = ((c[0] as u32) << 16) | ((*c.get(1).unwrap_or(&0) as u32) << 8) | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Run the pipeline here for one shader. `None` when there is no tool to run.
fn local_compile(stage: u32, source: &str, blend_key: u32) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tool = tools_binary()?;
    let stage = if stage == yakogl::shaderc::SHC_STAGE_VERTEX { "vert" } else { "frag" };
    let request = format!("{{\"stage\":\"{stage}\",\"blend_key\":{blend_key},\"source\":{}}}", json_string(source));
    let mut child =
        Command::new(tool).arg("shaderc-compile").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
    child.stdin.take()?.write_all(request.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Does the pipeline on this host produce what the server sent back? Then the
/// transport is faithful and it is the table that is out of date.
fn local_pipeline_agrees(stage: u32, source: &str, blend_key: u32, got: &ShcShader) -> Option<bool> {
    let reply = local_compile(stage, source, blend_key)?;
    let dw = json_number(&reply, "usc_dw")?;
    let blob = json_plain_string(&reply, "blob")?;
    Some(dw == got.usc_dw as usize && blob == b64_of_usc(got.usc_words()))
}

/// The rows to sweep: a spread-out sample of the table, or all of it.
fn rows_to_check() -> Vec<&'static ShcBlob> {
    let all: Vec<&ShcBlob> = SHC_BLOB_TABLE.iter().collect();
    let want = match std::env::var("YAKOGL_SHADERC_TEST_ROWS").as_deref() {
        Ok("all") => return all,
        Ok(n) => n.parse().unwrap_or(DEFAULT_ROWS),
        Err(_) => DEFAULT_ROWS,
    };
    if want >= all.len() {
        return all;
    }
    /* every k-th row, so the sample spans the whole table: the plain shaders,
     * the blend variants, the ES 3.00 set, the FemtoVG and the Qt shaders */
    let step = (all.len() / want).max(1);
    all.into_iter().step_by(step).take(want).collect()
}

fn skip(why: &str) {
    eprintln!("shaderc_remote: skipped ({why}); start tools/shaderc_server.py to run it");
}

#[test]
fn the_remote_back_end_reproduces_every_table_row_it_is_asked_for() {
    if !server_is_up() {
        return skip(&format!("no compile server at {}", server_addr()));
    }
    let rows = rows_to_check();
    let (mut checked, mut variants) = (0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    /* rows where the table is older than this host's compiler, not a remote bug */
    let mut stale: Vec<String> = Vec::new();

    for row in rows {
        let name = row.source_name();
        let Some(source) = source_of(row) else {
            /* a row whose text is not in SHC_SOURCE_TABLE cannot be asked for
             * by an application either, so there is nothing to compare */
            continue;
        };
        /* Every row is reached the way an application reaches it: compile the
         * source (always the plain shader), then ask for the variant. */
        let base = match compile(row.stage, source) {
            Ok(sh) => sh,
            Err((rc, log)) => {
                failures.push(format!("{name}: compile returned {rc}: {log}"));
                continue;
            }
        };
        let got = if row.blend_key == 0 {
            base
        } else {
            let mut out = ShcShader::ZERO;
            // SAFETY: a shader this back end handed out, and a valid output.
            let rc = unsafe { SHC_REMOTE.blend_variant.unwrap()(&base, row.blend_key, &mut out) };
            if rc != 0 {
                failures.push(format!("{name}: blend variant {:#x} returned {rc}", row.blend_key));
                continue;
            }
            variants += 1;
            out
        };
        let diff = differences(&row.shader, &got);
        if diff.is_empty() {
            checked += 1;
            continue;
        }
        /* The table disagrees. Ask the pipeline on this host who is right. */
        match local_pipeline_agrees(row.stage, source, row.blend_key, &got) {
            Some(true) => stale.push(format!("{name} (blend {:#x}): {}", row.blend_key, diff.join("; "))),
            Some(false) => failures.push(format!("{name} (blend {:#x}): {}", row.blend_key, diff.join("; "))),
            None => failures.push(format!(
                "{name} (blend {:#x}): {} [and yakogl-tools is not built, so this could not be \
                 checked against the pipeline on this host: cargo build -p yakogl-tools]",
                row.blend_key,
                diff.join("; ")
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "{} of the rows checked differ from the table AND from the pipeline on this host, so the \
         remote path lost or changed something:\n{}",
        failures.len(),
        failures.join("\n")
    );
    if !stale.is_empty() {
        /* Not this back end's bug, but nobody else would notice it either: an
         * offline build made on this host would not match blobs.rs. */
        eprintln!(
            "shaderc_remote: WARNING: {} of the rows checked differ from src/shaderc/blobs.rs, and \
             the offline pipeline on this host reproduces what the server sent, not what the table \
             holds. The table is NOT the thing to fix -- it is the silicon-verified one. This host's \
             pco_shc is missing something the table's had; on the lab host that was \
             tools/pco_shc/usclib-tex.cl.patch, present in the Mesa source but absent from the \
             built pco_usclib.spv, which only shows up in shaders that inline a usclib texture \
             routine (samplerCube, sampler2DArray, sampler3D, textureSize, texelFetch). See \
             tools/README.md, \"The pco_shc this host has\". The rows:\n{}",
            stale.len(),
            stale.join("\n")
        );
    }
    assert!(checked > 0, "no table row was checked: the table or the source table is empty");
    eprintln!(
        "shaderc_remote: {checked} rows identical to the table, {} reproduced by the pipeline here \
         but not in the table ({variants} blend variants asked for)",
        stale.len()
    );
}

#[test]
fn a_shader_the_compiler_refuses_comes_back_as_a_failure_with_its_log() {
    if !server_is_up() {
        return skip(&format!("no compile server at {}", server_addr()));
    }
    /* what glGetShaderInfoLog must end up holding: the compiler's own words */
    let bad = "precision mediump float;\nvoid main() { gl_FragColor = vec4(no_such_thing); }\n";
    let Err((rc, log)) = compile(SHC_STAGE_FRAGMENT, bad) else {
        panic!("a shader with an undeclared identifier must fail");
    };
    assert_eq!(rc, -yakogl::util::EINVAL, "a refused shader is a compile error, not a transport failure");
    assert!(log.contains("undeclared identifier"), "the glslang log must reach the application: {log}");
    assert!(log.contains("no_such_thing"), "and name what is wrong: {log}");
    /* the log names the shader, not the server's temp directory */
    assert!(!log.contains("/tmp/"), "the log must not carry a path on the compile host: {log}");
}

#[test]
fn a_refused_shader_is_remembered_so_the_suite_asks_once() {
    if !server_is_up() {
        return skip(&format!("no compile server at {}", server_addr()));
    }
    let bad = "precision mediump float;\nvoid main() { gl_FragColor = still_not_a_thing; }\n";
    let (Err(first), Err(again)) = (compile(SHC_STAGE_FRAGMENT, bad), compile(SHC_STAGE_FRAGMENT, bad)) else {
        panic!("a shader with an undeclared identifier must fail, both times");
    };
    assert_eq!(first, again, "the second compile of the same broken source gives the same answer");
}
