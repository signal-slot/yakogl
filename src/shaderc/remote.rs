// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shader back end that compiles on a host over TCP.
//!
//! The offline back end can only run shaders that were compiled into
//! `blobs.rs` at build time. The Khronos dEQP-GLES2 suite writes its shaders
//! as it runs, so there is nothing to precompile, and the PCO toolchain does
//! not exist on the target. This back end sends the GLSL text to
//! `tools/shaderc_server.py` on a host, which runs the very same pipeline that
//! wrote the offline table (`yakogl-tools shaderc-compile` -> glslang ->
//! pco_shc) and sends back the USC binary plus the reflection the table would
//! have held. What arrives is filled into the same [`ShcShader`] `offline.rs`
//! hands out, so everything above this file is unchanged.
//!
//! `std` is used here on purpose: sockets and a heap. The feature
//! `shaderc-remote` turns this module on and is only ever enabled for the
//! Linux host build (the dEQP host, and the board's Linux userspace over the
//! pvrgpu chardev). The Zephyr and bare-metal builds do not compile it, and
//! the portable GL core stays free of OS calls; see `SHADERC_NOTES.md`.
//!
//! Configuration (environment):
//!
//! | Variable | Meaning |
//! |---|---|
//! | `YAKOGL_SHADERC` | `remote` (the default here) or `offline` to force the table |
//! | `YAKOGL_SHADERC_SERVER` | `host:port` of the compile server (default `127.0.0.1:7777`) |
//! | `YAKOGL_SHADERC_TIMEOUT` | seconds to wait for a reply (default 120) |
//! | `YAKOGL_SHADERC_TOKEN` | added as the request's `"token"` member when set, to match `YAKOGL_SHADERC_TOKEN` on `tools/shaderc_server.py` |
//!
//! | `YAKOGL_SHADERC_REMOTE_CACHE` | records and link verdicts kept after their last user (default 64) |
//!
//! A compiled shader is owned by the GL objects that hold it: it lives while
//! a shader object or a linked program holds a copy, and the last
//! `glDeleteShader`/`glDeleteProgram` moves it to a bounded list of recently
//! freed records keyed by `{SHA-256 of the source, stage, blend key}`, so a
//! recompile of the same text is still served without a round trip. A shader
//! the server *refused* goes to that list too, with its log: an application
//! that recompiles the same broken source must not ask again. A server that
//! could not be reached is **not** cached — it may come up later.

use core::ffi::{c_char, c_void};
use core::fmt::Write as _;
use std::boxed::Box;
use std::collections::VecDeque;
use std::io::{Read, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::string::{String, ToString};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use std::vec::Vec;

use serde_json::Value;

use super::sha256::sha256;
use super::{ShcBackend, ShcShader, ShcUniform, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::util::{cstr_bytes, set_name, CStrWriter, EINVAL, EIO, ENOENT};

const DEFAULT_SERVER: &str = "127.0.0.1:7777";
const DEFAULT_PORT: &str = "7777";
const DEFAULT_TIMEOUT_S: u64 = 120;
/// The server caps a message at 16 MiB; refuse anything larger before
/// allocating for it.
const MAX_REPLY: u32 = 16 << 20;

/* ---- the compiled-shader cache ------------------------------------------ */

/// One shader the server answered for, owning everything the [`ShcShader`]
/// handed out points at.
///
/// Ownership is the GL core's reference count, the same one the online back
/// end keeps: every copy the GL core holds (the shader object's, a program's
/// snapshot at link, a blend variant's) is one [`retain`] or one handed-out
/// compile, and is paired with one [`release`]. While `refs > 0` the record is
/// in [`Cache::live`] and must not move or change, because those copies point
/// into its `usc`, `uniforms` and constant arrays and back at it through
/// `priv`. The record is boxed, so moving the `Box` between the lists does not
/// move what the copies point at.
///
/// The last release moves it to [`Cache::kept`], a bounded LRU of records no
/// one holds: a recompile of the same text is served from there without a
/// round trip, and the oldest is dropped when the list is full.
struct Compiled {
    /// SHA-256 of the GLSL text. The GPU family is not part of it because it
    /// is one per build ([`GPU_FAMILY`]): every record of a process is for
    /// the same family.
    sha: [u8; 32],
    stage: u32,
    blend_key: u32,
    /// Kept because a blend variant is the same text compiled again, and
    /// because the link check sends both stages' sources.
    source: String,
    usc: Vec<u32>,
    uniforms: Vec<ShcUniform>,
    /// The GC7000 constant file: what each slot holds, and the literal dwords.
    /// Two arrays the `ShcViv` points at, owned like `usc`.
    #[cfg(feature = "gpu-vivante")]
    const_contents: Vec<super::ShcVivConst>,
    #[cfg(feature = "gpu-vivante")]
    const_data: Vec<u32>,
    shader: ShcShader,
    /// Set when the server refused the shader; then `shader` is meaningless,
    /// nothing is handed out, and the record goes straight to the kept list.
    error: Option<String>,
    /// The GL core's copies of `shader`; 0 while in the kept list.
    refs: u32,
}

// SAFETY: the raw pointers in `shader` address the record's own heap arrays,
// which only move with the record's `Box` (they do not move at all); a record
// is only reached through the `CACHE` mutex.
unsafe impl Send for Compiled {}

/// Point a boxed record's `ShcShader` at its own arrays and at itself.
fn publish(rec: &mut Compiled) {
    rec.shader.usc = if rec.usc.is_empty() { core::ptr::null() } else { rec.usc.as_ptr() };
    rec.shader.usc_dw = rec.usc.len() as u32;
    rec.shader.uniforms = if rec.uniforms.is_empty() { core::ptr::null() } else { rec.uniforms.as_ptr() };
    rec.shader.num_uniforms = rec.uniforms.len() as u8;
    #[cfg(feature = "gpu-vivante")]
    {
        rec.shader.viv.const_contents =
            if rec.const_contents.is_empty() { core::ptr::null() } else { rec.const_contents.as_ptr() };
        rec.shader.viv.const_data = if rec.const_data.is_empty() { core::ptr::null() } else { rec.const_data.as_ptr() };
        rec.shader.viv.const_count = rec.const_contents.len() as u16;
    }
    rec.shader.priv_ = rec as *const Compiled as *const c_void;
}

/// One source pair the server has already link-checked.
struct Linked {
    vs: [u8; 32],
    fs: [u8; 32],
    /// The ES link log, or `None` when the pair links.
    error: Option<String>,
}

/// What a lookup or a fresh reply gives the caller. Returned once and taken
/// apart at once, so the size of the copy is not worth a heap allocation.
#[allow(clippy::large_enum_variant)]
enum Answer {
    /// A copy the caller now holds (one reference, one [`release`] owed).
    Shader(ShcShader),
    /// The server's log for a shader it refused.
    Refused(String),
}

/// Default of `YAKOGL_SHADERC_REMOTE_CACHE`: records kept after their last
/// holder let go, and link verdicts kept for pairs no live program uses.
const DEFAULT_KEPT: usize = 64;

/// `YAKOGL_SHADERC_REMOTE_CACHE`, read once.
fn kept_cap() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| env("YAKOGL_SHADERC_REMOTE_CACHE").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_KEPT))
}

/// The back end's records and link verdicts.
struct Cache {
    /// Records the GL core holds (`refs > 0`), in no order. Boxed on
    /// purpose: the GL core's copies point into a record and at it through
    /// `priv`, so a record must not move when this `Vec` grows.
    #[allow(clippy::vec_box)]
    live: Vec<Box<Compiled>>,
    /// Records no one holds, oldest first, at most [`kept_cap`].
    kept: VecDeque<Box<Compiled>>,
    /// Link verdicts, least recently used first. At most [`kept_cap`] plus
    /// the pairs whose two stages are both live (those are never evicted).
    linked: VecDeque<Linked>,
    live_peak: usize,
    linked_peak: usize,
}

/// The process-wide cache. A `Mutex` rather than a bare static because
/// `g_shaderc` is reachable from any thread that holds a context, even though
/// the GL core is single-threaded per context. The server round trip runs
/// without it.
static CACHE: Mutex<Cache> = Mutex::new(Cache {
    live: Vec::new(),
    kept: VecDeque::new(),
    linked: VecDeque::new(),
    live_peak: 0,
    linked_peak: 0,
});

/// The cache, also after a panic elsewhere left the mutex poisoned: every
/// method below leaves it consistent at each step.
fn cache() -> MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

impl Cache {
    fn live_index(&self, sh: &ShcShader) -> Option<usize> {
        if sh.priv_.is_null() {
            return None;
        }
        self.live.iter().position(|r| core::ptr::eq(&**r as *const Compiled as *const c_void, sh.priv_))
    }

    /// The live record behind `sh`, or `None` for a shader this back end did
    /// not hand out (the table's, the online compiler's, a zeroed one).
    fn live_of(&self, sh: &ShcShader) -> Option<&Compiled> {
        self.live_index(sh).map(|i| &*self.live[i])
    }

    /// A compile the cache can answer without the server.
    fn find(&mut self, sha: &[u8; 32], stage: u32, blend_key: u32) -> Option<Answer> {
        let hit = |r: &Compiled| r.stage == stage && r.blend_key == blend_key && &r.sha == sha;
        if let Some(r) = self.live.iter_mut().find(|r| hit(r)) {
            r.refs += 1;
            return Some(Answer::Shader(r.shader));
        }
        let i = self.kept.iter().position(|r| hit(r))?;
        let rec = self.kept.remove(i)?;
        Some(self.admit(rec))
    }

    /// Hand out a record: a compiled one becomes live with the caller as its
    /// one holder, a refusal goes to the back of the kept list.
    fn admit(&mut self, mut rec: Box<Compiled>) -> Answer {
        if let Some(log) = &rec.error {
            let answer = Answer::Refused(log.clone());
            self.keep(rec);
            return answer;
        }
        rec.refs = 1;
        let answer = Answer::Shader(rec.shader);
        self.live.push(rec);
        self.live_peak = self.live_peak.max(self.live.len());
        answer
    }

    fn keep(&mut self, rec: Box<Compiled>) {
        debug_assert_eq!(rec.refs, 0, "a record someone holds is not kept");
        self.kept.push_back(rec);
        while self.kept.len() > kept_cap() {
            self.kept.pop_front();
        }
    }

    fn retain(&mut self, sh: &ShcShader) {
        if let Some(i) = self.live_index(sh) {
            self.live[i].refs += 1;
        }
    }

    fn release(&mut self, sh: &ShcShader) {
        let Some(i) = self.live_index(sh) else {
            return;
        };
        let rec = &mut self.live[i];
        debug_assert!(rec.refs > 0, "a live record with no holders");
        rec.refs = rec.refs.saturating_sub(1);
        if rec.refs == 0 {
            let rec = self.live.swap_remove(i);
            self.keep(rec);
            self.trim_linked();
        }
    }

    /// Whether a live program can be using the pair: both stages are live.
    fn pair_in_use(&self, l: &Linked) -> bool {
        let live = |sha: &[u8; 32], stage: u32| self.live.iter().any(|r| r.stage == stage && &r.sha == sha);
        live(&l.vs, SHC_STAGE_VERTEX) && live(&l.fs, SHC_STAGE_FRAGMENT)
    }

    /// Evict the least recently used pairs past the cap, skipping the ones in
    /// use: a relink of a live program must stay free.
    fn trim_linked(&mut self) {
        let mut i = 0;
        while self.linked.len() > kept_cap() && i < self.linked.len() {
            if self.pair_in_use(&self.linked[i]) {
                i += 1;
            } else {
                self.linked.remove(i);
            }
        }
    }

    /// A pair's verdict, now the most recently used; `None` when unchecked.
    fn linked_find(&mut self, vs: &[u8; 32], fs: &[u8; 32]) -> Option<Option<String>> {
        let i = self.linked.iter().position(|l| &l.vs == vs && &l.fs == fs)?;
        let l = self.linked.remove(i)?;
        let verdict = l.error.clone();
        self.linked.push_back(l);
        Some(verdict)
    }

    fn linked_put(&mut self, l: Linked) {
        self.linked.push_back(l);
        self.trim_linked();
        self.linked_peak = self.linked_peak.max(self.linked.len());
    }
}

/// The GL core keeps one more copy of `sh` (see `shaderc::retain_shader`).
/// Nothing for a shader this back end did not hand out.
pub fn retain(sh: &ShcShader) {
    cache().retain(sh);
}

/// The GL core lets go of one copy of `sh`. The last one moves the record to
/// the kept list, where no `ShcShader` points at it any more.
pub fn release(sh: &ShcShader) {
    cache().release(sh);
}

/// What the cache holds now and the most it has held: the remote back end's
/// high-water line (the online back end's is `shc_online_keep_stats`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheStats {
    /// Records the GL core holds, and the most ever held at once.
    pub live: usize,
    pub live_peak: usize,
    /// Records no one holds, kept for a recompile, and their cap.
    pub kept: usize,
    pub kept_cap: usize,
    /// Link verdicts kept, and the most ever kept at once.
    pub linked: usize,
    pub linked_peak: usize,
}

impl core::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "remote shaderc: {} live (peak {}), {} kept of {}, {} link verdicts (peak {})",
            self.live, self.live_peak, self.kept, self.kept_cap, self.linked, self.linked_peak
        )
    }
}

/// See [`CacheStats`].
pub fn cache_stats() -> CacheStats {
    let c = cache();
    CacheStats {
        live: c.live.len(),
        live_peak: c.live_peak,
        kept: c.kept.len(),
        kept_cap: kept_cap(),
        linked: c.linked.len(),
        linked_peak: c.linked_peak,
    }
}

/// Print [`cache_stats`] as one line on stderr, for a harness: call it at
/// exit, or before and after a create/compile/delete churn, and the live
/// figure must come back to where it started. Returns 0.
#[no_mangle]
pub extern "C" fn shc_remote_cache_report() -> i32 {
    std::eprintln!("{}", cache_stats());
    0
}

/* ---- talking to the server ---------------------------------------------- */

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn timeout() -> Duration {
    Duration::from_secs(env("YAKOGL_SHADERC_TIMEOUT").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_TIMEOUT_S))
}

/// Add the `"token"` member `tools/shaderc_server.py` reads from
/// `YAKOGL_SHADERC_TOKEN` (`TOKEN_FIELD` there), when the client's own copy
/// of that variable is set. The server strips the field before hashing its
/// cache key, so it never becomes part of what the compile is keyed on; when
/// the variable is unset here, the request is exactly what it was before
/// this field existed.
fn add_token(request: &mut Value) {
    if let Some(token) = env("YAKOGL_SHADERC_TOKEN") {
        if let Value::Object(map) = request {
            map.insert("token".to_string(), Value::String(token));
        }
    }
}

/// `YAKOGL_SHADERC_SERVER`, with the default port added when it names only a
/// host (`lab-host` -> `lab-host:7777`).
fn server_addr() -> String {
    #[cfg(test)]
    if let Some(addr) = tests::fake_server_addr() {
        return addr;
    }
    let spec = env("YAKOGL_SHADERC_SERVER").unwrap_or_else(|| DEFAULT_SERVER.to_string());
    if spec.rfind(':').is_some_and(|i| i > 0) {
        spec
    } else {
        std::format!("{spec}:{DEFAULT_PORT}")
    }
}

fn connect() -> Result<TcpStream, String> {
    let addr = server_addr();
    let tmo = timeout();
    let mut last = String::new();
    let addrs = match addr.to_socket_addrs() {
        Ok(a) => a,
        Err(e) => return Err(std::format!("cannot resolve {addr}: {e}")),
    };
    for a in addrs {
        match TcpStream::connect_timeout(&a, tmo) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(tmo));
                let _ = s.set_write_timeout(Some(tmo));
                return Ok(s);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(std::format!(
        "cannot reach the compile server at {addr} ({last}); \
         set YAKOGL_SHADERC_SERVER, and start tools/shaderc_server.py"
    ))
}

/// Send one request and read the reply. The framing is a 4-byte big-endian
/// byte count then that many bytes of UTF-8 JSON, in both directions.
fn call(request: &str) -> Result<String, String> {
    let mut sock = connect()?;
    let len = u32::try_from(request.len()).map_err(|_| "the request is larger than the protocol allows".to_string())?;
    sock.write_all(&len.to_be_bytes())
        .and_then(|()| sock.write_all(request.as_bytes()))
        .map_err(|e| std::format!("sending the request failed: {e}"))?;

    let mut hdr = [0u8; 4];
    sock.read_exact(&mut hdr).map_err(|e| std::format!("no reply from the compile server: {e}"))?;
    let n = u32::from_be_bytes(hdr);
    if n == 0 || n > MAX_REPLY {
        return Err(std::format!("reply length {n} out of range"));
    }
    let mut body = std::vec![0u8; n as usize];
    sock.read_exact(&mut body).map_err(|e| std::format!("short reply from the compile server: {e}"))?;
    String::from_utf8(body).map_err(|e| std::format!("the reply is not UTF-8: {e}"))
}

/* ---- the reply -> struct shc_shader ------------------------------------- */

fn as_u32(v: Option<&Value>) -> u32 {
    v.and_then(Value::as_u64).unwrap_or(0) as u32
}

fn as_str(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or("")
}

fn name_field<const N: usize>(field: &mut [c_char; N], v: Option<&Value>) {
    set_name(field, as_str(v).as_bytes());
}

/// Decode the base64 the shader binary travels as. `None` on a stray
/// character, which means a reply this back end must not trust.
fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let d = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | d as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// The reply's `reflection` object filled into a [`ShcShader`], field for
/// field with what the offline table holds. `Err` names what was wrong.
fn fill_shader(reply: &Value, rec: &mut Compiled) -> Result<(), String> {
    let refl = reply.get("reflection").ok_or_else(|| "reply has no reflection".to_string())?;
    let usc_dw = as_u32(reply.get("usc_dw")) as usize;
    let blob = reply.get("blob").and_then(Value::as_str).ok_or_else(|| "reply has no USC binary".to_string())?;
    let raw = b64_decode(blob).ok_or_else(|| "the USC binary is not base64".to_string())?;
    if usc_dw == 0 || raw.len() != usc_dw * 4 {
        return Err(std::format!("the USC binary is {} bytes, not the {usc_dw} dwords announced", raw.len()));
    }
    rec.usc = raw.as_chunks::<4>().0.iter().copied().map(u32::from_le_bytes).collect();

    let sh = &mut rec.shader;
    sh.temps = as_u32(refl.get("temps")) as u16;
    sh.uses_w = as_u32(refl.get("uses_w")) as u8;
    sh.uses_discard = as_u32(refl.get("uses_discard")) as u8;
    sh.reads_front_facing = as_u32(refl.get("reads_front_facing")) as u8;
    sh.meta_off = as_u32(refl.get("meta_off")) as u16;
    sh.fs_front_face_off1 = as_u32(refl.get("fs_front_face_off1")) as u16;
    sh.num_outputs = as_u32(refl.get("num_outputs")) as u8;
    sh.num_tile_buffers = as_u32(refl.get("num_tile_buffers")) as u8;
    sh.tile_buffer_sh = as_u32(refl.get("tile_buffer_sh")) as u16;
    sh.vs_output_size = as_u32(refl.get("vs_output_size")) as u8;
    sh.vs_psize_off = as_u32(refl.get("vs_psize_off")) as u8;
    sh.vs_vertex_id_reg1 = as_u32(refl.get("vs_vertex_id_reg1")) as u8;
    sh.vs_instance_id_reg1 = as_u32(refl.get("vs_instance_id_reg1")) as u8;
    sh.uniform_dwords = as_u32(refl.get("uniform_dwords")) as u16;
    sh.uniform_sh_dwords = as_u32(refl.get("uniform_sh_dwords")) as u16;
    sh.blend_key = as_u32(refl.get("blend_key"));
    sh.uses_fbfetch = as_u32(refl.get("uses_fbfetch")) as u8;
    sh.uses_pntc = as_u32(refl.get("uses_pntc")) as u8;
    sh.uses_z = as_u32(refl.get("uses_z")) as u8;
    sh.fragcoord_ytf_off1 = as_u32(refl.get("fragcoord_ytf_off1")) as u16;
    sh.native_cube = as_u32(refl.get("native_cube")) as u8;
    /* NOT `as_u32`, whose default is 0: this field's "the shader has no blend
     * constants" is the sentinel SHC_NO_BLEND_CONSTS, and 0 is a real offset --
     * dword 0, where the first uniform of a block usually sits.
     *
     * A server that omits the field (ours does, for every shader without blend
     * constants) therefore used to hand back "blend constants at dword 0", and
     * the draw path dutifully wrote glBlendColor over the application's first
     * uniform. Default (0, 0, 0, 0) against gl_test case 1: every colour came
     * out transparent black while the geometry, fed by vertex uniforms, stayed
     * correct. Six cases on the i.MX8MP failed only under this back end, with
     * code, constants and reflection byte-identical to the offline table.
     *
     * The other scalars above all take 0 as their genuine "none" (the `*_off1`
     * fields are 1-based for exactly that reason); this is the one sentinel,
     * so it is the one that needs an explicit default. */
    sh.blend_consts_off = match refl.get("blend_consts_off").and_then(Value::as_u64) {
        Some(v) => v as u8,
        None => super::SHC_NO_BLEND_CONSTS,
    };

    let empty = Vec::new();
    let varyings = refl.get("varyings").and_then(Value::as_array).unwrap_or(&empty);
    if varyings.len() > sh.varyings.len() {
        return Err(std::format!("{} varyings, shc_shader holds {}", varyings.len(), sh.varyings.len()));
    }
    for (d, v) in sh.varyings.iter_mut().zip(varyings) {
        d.location = as_u32(v.get("location")) as u8;
        d.components = as_u32(v.get("components")) as u8;
        d.interp = as_u32(v.get("interp")) as u8;
        d.vtxout_off = as_u32(v.get("vtxout_off")) as u8;
        name_field(&mut d.name, v.get("name"));
    }
    sh.num_varyings = varyings.len() as u8;

    let attribs = refl.get("attribs").and_then(Value::as_array).unwrap_or(&empty);
    if attribs.len() > sh.attribs.len() {
        return Err(std::format!("{} vertex attributes, shc_shader holds {}", attribs.len(), sh.attribs.len()));
    }
    for (d, a) in sh.attribs.iter_mut().zip(attribs) {
        d.location = as_u32(a.get("location")) as u8;
        d.components = as_u32(a.get("components")) as u8;
        d.vi_reg = as_u32(a.get("vi_reg")) as u8;
        d.format = as_u32(a.get("format")) as u8;
        d.gl_type = as_u32(a.get("gl_type")) as u16;
        name_field(&mut d.name, a.get("name"));
    }
    sh.num_attribs = attribs.len() as u8;

    let uniforms = refl.get("uniforms").and_then(Value::as_array).unwrap_or(&empty);
    if uniforms.len() > u8::MAX as usize {
        return Err(std::format!("{} uniforms, shc_shader counts {}", uniforms.len(), u8::MAX));
    }
    rec.uniforms = uniforms
        .iter()
        .map(|u| {
            let mut d = ShcUniform::ZERO;
            name_field(&mut d.name, u.get("name"));
            d.dword_off = as_u32(u.get("dword_off")) as u16;
            d.dwords = as_u32(u.get("dwords")) as u16;
            d.gl_type = as_u32(u.get("gl_type")) as u16;
            d.array_len = as_u32(u.get("array_len")) as u8;
            d.binding = as_u32(u.get("binding")) as u8;
            d.inner = as_u32(u.get("inner")) as u8;
            d
        })
        .collect();
    #[cfg(feature = "gpu-vivante")]
    parse_viv(refl, rec)?;
    Ok(())
}

/// The GC7000 half of the reflection: which register each input and output
/// lives in, where the fixed outputs go, and the constant file.
///
/// Without this the shader arrives with correct code and no wiring, which
/// draws nothing recognisable and looks exactly like a code-generation bug --
/// it cost a day's hunting through blobs that turned out to be identical.
/// `ShcViv` is only in the struct on a `gpu-vivante` build, so this is too.
#[cfg(feature = "gpu-vivante")]
fn parse_viv(refl: &Value, rec: &mut Compiled) -> Result<(), String> {
    use super::{ShcVivIo, SHC_VIV_MAX_IO};

    let viv = match refl.get("viv") {
        Some(v) => v,
        /* A PowerVR reply to a Vivante build: the server answered for the
         * wrong GPU, which is worth saying plainly rather than rendering
         * nothing and leaving someone to find out from the pixels. */
        None => return Err("the reply carries no GC7000 reflection (is the server compiling for this GPU?)".into()),
    };
    let empty = std::vec::Vec::new();
    let io = |key: &str| -> &std::vec::Vec<Value> { viv.get(key).and_then(Value::as_array).unwrap_or(&empty) };

    for (which, dst, count) in [
        ("in", &mut rec.shader.viv.in_ as *mut [ShcVivIo; SHC_VIV_MAX_IO], 0usize),
        ("out", &mut rec.shader.viv.out as *mut [ShcVivIo; SHC_VIV_MAX_IO], 1usize),
    ] {
        let list = io(which);
        if list.len() > SHC_VIV_MAX_IO {
            return Err(std::format!("{} {which} registers, shc_viv holds {}", list.len(), SHC_VIV_MAX_IO));
        }
        // SAFETY: `dst` is a field of `rec.shader.viv`, borrowed for this loop
        // only; the two arms name different fields.
        let slots = unsafe { &mut *dst };
        for (d, e) in slots.iter_mut().zip(list) {
            d.reg = as_u32(e.get("reg")) as u8;
            d.slot = as_u32(e.get("slot")) as u8;
            d.components = as_u32(e.get("components")) as u8;
            d.interp = as_u32(e.get("interp")) as u8;
            name_field(&mut d.name, e.get("name"));
        }
        if count == 0 {
            rec.shader.viv.num_in = list.len() as u8;
        } else {
            rec.shader.viv.num_out = list.len() as u8;
        }
    }

    let reg = |key: &str| -> u8 {
        match viv.get(key).and_then(Value::as_i64) {
            /* -1 on the wire is "this shader has none"; the struct spells
             * that SHC_VIV_NO_REG. */
            Some(v) if v < 0 => super::SHC_VIV_NO_REG,
            Some(v) => v as u8,
            None => super::SHC_VIV_NO_REG,
        }
    };
    rec.shader.viv.vs_pos_out_reg = reg("vs_pos_out_reg");
    rec.shader.viv.vs_pointsize_out_reg = reg("vs_pointsize_out_reg");
    rec.shader.viv.vs_id_in_reg = reg("vs_id_in_reg");
    rec.shader.viv.ps_color_out_reg = reg("ps_color_out_reg");
    rec.shader.viv.ps_depth_out_reg = reg("ps_depth_out_reg");
    rec.shader.viv.vs_load_balancing = as_u32(viv.get("vs_load_balancing"));
    rec.shader.viv.input_count_unk8 = as_u32(viv.get("input_count_unk8")) as u16;

    /* The constant file: `const_contents` says what each slot is, `const_data`
     * holds the literals. They are parallel, and a mismatch is the server's
     * bug rather than the shader's. */
    let contents = refl.get("const_contents").and_then(Value::as_array).unwrap_or(&empty);
    let data = refl.get("const_data").and_then(Value::as_array).unwrap_or(&empty);
    if !contents.is_empty() && !data.is_empty() && contents.len() != data.len() {
        return Err(std::format!("const_contents has {} entries, const_data {}", contents.len(), data.len()));
    }
    rec.const_contents = contents.iter().map(|c| as_u32(Some(c)) as super::ShcVivConst).collect();
    rec.const_data = data.iter().map(|d| as_u32(Some(d))).collect();
    Ok(())
}

/* ---- one compile --------------------------------------------------------- */

fn stage_name(stage: u32) -> &'static str {
    if stage == SHC_STAGE_VERTEX {
        "vert"
    } else {
        "frag"
    }
}

/// What a reply says about the shader, and about the host.
///
/// The two failures must never be confused. A shader the compiler rejected is
/// the application's business: remember it and report `GL_COMPILE_STATUS`
/// false with the log, which is what dEQP-GLES2's negative cases read. A host
/// that could not compile at all is not: do not remember it (it may work in a
/// minute) and do not tell the application its shader is bad.
///
/// Ruled and settled, for the name pool that #47 brings here: A name that does not fit the pool is a deterministic property of
/// the shader, so it comes back as `{"ok": false, "error": ...}`,
/// `GL_COMPILE_STATUS` false with the message in the info log, and it is cached
/// as a refusal; a service error is reserved for the host being unable to
/// answer at all (a missing compiler, an unreachable server).
///
/// The difference on the wire is whether `"ok"` is there at all — a reply with
/// an `"error"` and no `"ok"` is the service saying it is broken. `"log"` is
/// accepted beside `"error"` for the log text, so the two names the pipeline
/// side may use both arrive.
enum Verdict {
    Compiled,
    ShaderRefused(String),
    ServiceBroken(String),
}

fn refusal(reply: &Value) -> Verdict {
    let text = |key: &str| reply.get(key).and_then(Value::as_str).unwrap_or("");
    let log = || {
        let log = if text("error").is_empty() { text("log") } else { text("error") };
        if log.is_empty() {
            "the compile server refused the shader".to_string()
        } else {
            log.to_string()
        }
    };
    match reply.get("ok").and_then(Value::as_bool) {
        Some(true) => Verdict::Compiled,
        Some(false) => Verdict::ShaderRefused(log()),
        None => Verdict::ServiceBroken(if text("error").is_empty() {
            "the compile server sent a reply with no verdict in it".to_string()
        } else {
            text("error").to_string()
        }),
    }
}

/// The GPU family this build compiles for, as the protocol names it.
///
/// The server routes the request on this and includes it in its cache key, so
/// asking for the wrong family is not an error anywhere -- it is a shader
/// binary for the other GPU, handed to a backend that will run it. Hence one
/// constant from the build's own feature, rather than a string at each call
/// site: it was "pvr" in both, which is exactly the bug this shape prevents.
const GPU_FAMILY: &str = if cfg!(feature = "gpu-vivante") { "vivante" } else { "pvr" };

/// The shader for `{sha, stage, blend_key}`, from the cache or compiled on the
/// server first. A [`Answer::Shader`] is a copy the caller holds. `Err` is a
/// server that could not be reached or answered nonsense — not cached,
/// because it may come up later.
fn fetch(sha: &[u8; 32], stage: u32, glsl: &str, blend_key: u32) -> Result<Answer, String> {
    if let Some(answer) = cache().find(sha, stage, blend_key) {
        return Ok(answer);
    }
    let mut request = serde_json::json!({
        "gpu": GPU_FAMILY,
        "op": "compile",
        "stage": stage_name(stage),
        "blend_key": blend_key,
        "source": glsl,
    });
    add_token(&mut request);
    let body = call(&request.to_string())?;
    let reply: Value = serde_json::from_str(&body).map_err(|e| std::format!("the reply is not JSON: {e}"))?;

    let mut rec = Compiled {
        sha: *sha,
        stage,
        blend_key,
        source: glsl.to_string(),
        usc: Vec::new(),
        uniforms: Vec::new(),
        #[cfg(feature = "gpu-vivante")]
        const_contents: Vec::new(),
        #[cfg(feature = "gpu-vivante")]
        const_data: Vec::new(),
        shader: ShcShader::ZERO,
        error: None,
        refs: 0,
    };
    match refusal(&reply) {
        /* a reply this back end cannot use is the host's problem, not the
         * application's, so it is reported as a transport failure and not
         * remembered as a compile error */
        Verdict::Compiled => fill_shader(&reply, &mut rec)?,
        Verdict::ShaderRefused(log) => rec.error = Some(log),
        Verdict::ServiceBroken(why) => return Err(why),
    }
    let mut rec = Box::new(rec);
    publish(&mut rec);
    Ok(cache().admit(rec))
}

/* ---- the back end entry points ------------------------------------------ */

unsafe extern "C" fn remote_compile(
    stage: u32,
    glsl: *const c_char,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    // SAFETY: the caller passes a null log or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };

    if glsl.is_null() || out.is_null() {
        let _ = log.write_str("remote shaderc: NULL source or output");
        return -EINVAL;
    }
    // SAFETY: glsl is a non-null NUL-terminated string (the GL layer's copy).
    let bytes = unsafe { cstr_bytes(glsl) };
    let digest = sha256(bytes);
    let text = match core::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => {
            let _ = write!(log, "remote shaderc: the shader source is not UTF-8: {e}");
            return -EINVAL;
        }
    };

    match fetch(&digest, stage, text, 0) {
        /* the server could not be reached: -EIO, and the log says how to fix
         * it. Not a compile error, so it is not remembered. */
        Err(why) => {
            let _ = write!(log, "remote shaderc: {why}");
            -EIO
        }
        /* a real compile error: GL_COMPILE_STATUS false with the
         * glslang/pco_shc log the application asked for */
        Ok(Answer::Refused(text)) => {
            let _ = log.write_str(&text);
            -EINVAL
        }
        /* the shader object is the copy's holder, and releases it */
        Ok(Answer::Shader(sh)) => {
            // SAFETY: out is non-null and points at the caller's shc_shader.
            unsafe { out.write(sh) };
            0
        }
    }
}

/// The FS compiled for one blend state. This GPU has no blend unit, so the
/// blend arithmetic is lowered into the shader: the server compiles a variant
/// of the same source for the key the GL core built from the draw state.
unsafe extern "C" fn remote_blend_variant(fs: *const ShcShader, blend_key: u32, out: *mut ShcShader) -> i32 {
    if fs.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: fs points at a shc_shader; read by value because out may be the
    // same object.
    let fs = unsafe { fs.read() };
    let (sha, source) = {
        let mut c = cache();
        let Some(base) = c.live_of(&fs) else {
            return -EINVAL;
        };
        let key = (base.sha, base.source.clone());
        if blend_key == 0 {
            /* a copy the caller releases like any variant */
            c.retain(&fs);
            // SAFETY: out is non-null and points at the caller's shc_shader.
            unsafe { out.write(fs) };
            return 0;
        }
        key
    };
    /* the GL core then draws unblended and logs */
    let Ok(Answer::Shader(mut sh)) = fetch(&sha, SHC_STAGE_FRAGMENT, &source, blend_key) else {
        return -ENOENT;
    };
    /* The variant comes from the same source, so it has the same varyings,
     * but the link patched the program's FS copy to the vertex shader's output
     * offsets and this fresh copy has not been patched. Carry those offsets
     * over, or the coefficient program would iterate the wrong vtxout dwords. */
    let patched = &fs.varyings[..(fs.num_varyings as usize).min(fs.varyings.len())];
    let n = (sh.num_varyings as usize).min(sh.varyings.len());
    for v in sh.varyings[..n].iter_mut() {
        if let Some(p) = patched.iter().find(|p| p.location == v.location) {
            v.vtxout_off = p.vtxout_off;
        }
    }
    // SAFETY: out is non-null and points at the caller's shc_shader.
    unsafe { out.write(sh) };
    0
}

/// One stage of a pair to link-check: its source's digest and text.
struct Stage {
    sha: [u8; 32],
    source: String,
}

/// Ask the server to apply the ES link rules to the two **sources**, which the
/// compiled shaders no longer carry (`mediump vec3` and `highp vec3` compile
/// to the same thing). Cached per source pair. A server that cannot be reached
/// is not an error here: the wiring check still runs, and `compile` has
/// already told the application about the unreachable server.
fn es_link_check(vs: &Stage, fs: &Stage, log: &mut CStrWriter) -> bool {
    let report = |err: &Option<String>, log: &mut CStrWriter| match err {
        Some(text) => {
            let _ = log.write_str(text);
            false
        }
        None => true,
    };
    if let Some(hit) = cache().linked_find(&vs.sha, &fs.sha) {
        return report(&hit, log);
    }
    /* The host tool is built for no GPU, so the varying budget it packs
     * against (GLSL ES 1.00 Appendix A 7) is this build's GL_MAX_VARYING_VECTORS. */
    let mut request = serde_json::json!({"gpu": GPU_FAMILY, "op": "link", "vert": vs.source, "frag": fs.source,
                                         "max_varying_vectors": crate::gl::context::MAX_VARYING_VECTORS});
    add_token(&mut request);
    let Ok(body) = call(&request.to_string()) else { return true };
    let Ok(reply) = serde_json::from_str::<Value>(&body) else { return true };
    let err = match refusal(&reply) {
        Verdict::Compiled => None,
        Verdict::ShaderRefused(log) => Some(log),
        /* the host cannot run the check; that is not a link error, and the
         * wiring check below still runs */
        Verdict::ServiceBroken(_) => return true,
    };
    cache().linked_put(Linked { vs: vs.sha, fs: fs.sha, error: err.clone() });
    report(&err, log)
}

unsafe extern "C" fn remote_link(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32 {
    if !vs.is_null() && !fs.is_null() {
        // SAFETY: both point at valid distinct shc_shader objects.
        let (v, f) = unsafe { (&*vs, &*fs) };
        let pair = {
            let c = cache();
            let stage = |r: &Compiled| Stage { sha: r.sha, source: r.source.clone() };
            c.live_of(v).map(stage).zip(c.live_of(f).map(stage))
        };
        if let Some((v, f)) = pair {
            // SAFETY: the caller passes a null log or one valid for log_sz bytes.
            let mut w = unsafe { CStrWriter::new(log, log_sz) };
            if !es_link_check(&v, &f, &mut w) {
                return -EINVAL;
            }
        }
    }
    /* then the wiring both back ends share: pair the varyings by name and
     * bounds-check them against the VS output vector */
    // SAFETY: the caller's contract, passed straight through.
    /* One rule per GPU, the same one the offline back end applies: the
     * PowerVR pairs the stages through the VS output vector, the GC7000 by
     * register. Getting this wrong is not a subtle failure -- the PowerVR
     * check reads vs_output_size, which is unused and always 0 on the
     * GC7000, so every program failed to link with "VS emits no
     * gl_Position" until this was split. */
    #[cfg(feature = "gpu-vivante")]
    // SAFETY: the caller's contract, passed straight through.
    return unsafe { super::link::link_shaders_viv(vs, fs, log, log_sz, "remote shaderc") };
    #[cfg(not(feature = "gpu-vivante"))]
    // SAFETY: the caller's contract, passed straight through.
    unsafe {
        super::link::link_shaders(vs, fs, log, log_sz, "remote shaderc")
    }
}

/// The remote back end: compile on a host, wire up here.
pub static SHC_REMOTE: ShcBackend = ShcBackend {
    compile: Some(remote_compile),
    link: Some(remote_link),
    blend_variant: Some(remote_blend_variant),
    compile_vs_linked: None,
    lod_sensitivity: None,
    sampler_variant: None,
};

/// Registration hook, matching `shc_offline_register()`: lets an application
/// select this back end explicitly.
///
/// # Safety
/// Call before the first GL call, and from one thread (as for the whole
/// library).
#[no_mangle]
pub unsafe extern "C" fn shc_remote_register() -> *const ShcBackend {
    // SAFETY: single-threaded by contract; a plain store, no reference formed.
    unsafe { super::g_shaderc = &SHC_REMOTE };
    &SHC_REMOTE
}

/// Drop what no one holds (tests): the kept records and the link verdicts,
/// so the next compile of a kept source asks the server again. Live records
/// stay, because the GL core's copies point into them.
#[cfg(test)]
fn cache_clear() {
    let mut c = cache();
    c.kept.clear();
    c.linked.clear();
}

/* ---- which back end this build starts with ------------------------------ */

/// `YAKOGL_SHADERC=offline|remote`, as in the C. Applied once, before the
/// first use of `g_shaderc`, so an application can still call
/// `shc_offline_register()` or `shc_remote_register()` afterwards and be
/// obeyed.
///
/// A build that compiled this module in is a host build and defaults to
/// `remote` — that is the reason to have compiled it in. The exception is the
/// unit-test harness (`host-test`), which has no compile server and whose
/// tests are written against the offline table: there `remote` is opt-in, and
/// `tests/shaderc_remote.rs` is what opts in.
pub fn select_once() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let remote = match env("YAKOGL_SHADERC").as_deref() {
            Some("offline") => false,
            Some("remote") => true,
            _ => !cfg!(feature = "host-test"),
        };
        if remote {
            // SAFETY: single-threaded by contract, and Once makes this the
            // only store; a plain store, no reference to the static is formed.
            unsafe { super::g_shaderc = &SHC_REMOTE };
        }
    });
}

#[cfg(test)]
mod tests {
    //! The wire decoding, with no server: what the back end does with a reply
    //! is the part a unit test can pin. That the server actually reproduces
    //! every table row is `tests/shaderc_remote.rs`.
    use super::*;
    use crate::util::name_bytes;
    use std::format;

    fn reply(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    fn blank() -> Compiled {
        Compiled {
            sha: [0; 32],
            stage: SHC_STAGE_FRAGMENT,
            blend_key: 0,
            source: String::new(),
            usc: Vec::new(),
            uniforms: Vec::new(),
            #[cfg(feature = "gpu-vivante")]
            const_contents: Vec::new(),
            #[cfg(feature = "gpu-vivante")]
            const_data: Vec::new(),
            shader: ShcShader::ZERO,
            error: None,
            refs: 0,
        }
    }

    /// The tests that read or change the process-wide cache run one at a
    /// time: its counts are what they assert.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hand a finished record out through the cache, as `fetch` does after a
    /// reply: the caller holds the returned copy.
    fn admit_for_test(rec: Compiled) -> ShcShader {
        let mut rec = Box::new(rec);
        publish(&mut rec);
        match cache().admit(rec) {
            Answer::Shader(sh) => sh,
            Answer::Refused(log) => panic!("a refused record was admitted: {log}"),
        }
    }

    /// How many copies the GL core holds of the record behind `sh`; `None`
    /// when no live record is behind it.
    fn holders(sh: &ShcShader) -> Option<u32> {
        cache().live_of(sh).map(|r| r.refs)
    }

    /* ---- a compile server in this process ----
     *
     * Started on the first use and kept for the process; `server_addr` asks
     * here first. It answers every compile with a one-dword shader whose
     * reflection passes this build's link rule (a VS with gl_Position, the
     * GC7000 wiring when that is the build), refuses a source that contains
     * "BROKEN", and says every pair links. It counts compile requests, which
     * is how a test tells a cache hit from a round trip. */

    static FAKE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static FAKE_COMPILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    /// Whether the most recent request the fake server decoded carried a
    /// `"token"` member, the way `tools/shaderc_server.py` would check it.
    static FAKE_LAST_HAD_TOKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub(super) fn fake_server_addr() -> Option<String> {
        FAKE.get().cloned()
    }

    fn fake_reply(req: &Value) -> String {
        FAKE_LAST_HAD_TOKEN.store(req.get("token").is_some(), std::sync::atomic::Ordering::SeqCst);
        if as_str(req.get("op")) == "link" {
            return r#"{"ok":true}"#.to_string();
        }
        FAKE_COMPILES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if as_str(req.get("source")).contains("BROKEN") {
            return r#"{"ok":false,"error":"fake: BROKEN is not GLSL"}"#.to_string();
        }
        let vert = as_str(req.get("stage")) == "vert";
        serde_json::json!({
            "ok": true, "usc_dw": 1, "blob": "AQAAAA==",
            "reflection": {
                "vs_output_size": if vert { 4 } else { 0 },
                "viv": {"in": [], "out": [],
                        "vs_pos_out_reg": if vert { 0 } else { -1 }, "vs_pointsize_out_reg": -1,
                        "vs_id_in_reg": -1, "ps_color_out_reg": if vert { -1 } else { 0 },
                        "ps_depth_out_reg": -1, "vs_load_balancing": 0, "input_count_unk8": 0},
            },
        })
        .to_string()
    }

    fn fake_serve(mut s: TcpStream) {
        let mut hdr = [0u8; 4];
        if s.read_exact(&mut hdr).is_err() {
            return;
        }
        let mut body = std::vec![0u8; u32::from_be_bytes(hdr) as usize];
        if s.read_exact(&mut body).is_err() {
            return;
        }
        let reply = fake_reply(&serde_json::from_slice(&body).unwrap_or(Value::Null));
        let _ = s.write_all(&(reply.len() as u32).to_be_bytes()).and_then(|()| s.write_all(reply.as_bytes()));
    }

    /// Start the server once; the compile requests it has answered so far.
    fn fake_server() -> usize {
        FAKE.get_or_init(|| {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = l.local_addr().unwrap().to_string();
            std::thread::spawn(move || {
                for s in l.incoming().flatten() {
                    fake_serve(s);
                }
            });
            addr
        });
        FAKE_COMPILES.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// One compile through the back end's entry point: the shader, or the log.
    fn compile(stage: u32, text: &str) -> Result<ShcShader, String> {
        let src = std::ffi::CString::new(text).unwrap();
        let mut out = ShcShader::ZERO;
        let mut log = [0 as c_char; 256];
        // SAFETY: valid source, output and log buffer.
        let rc = unsafe { remote_compile(stage, src.as_ptr(), &mut out, log.as_mut_ptr(), log.len()) };
        if rc == 0 {
            Ok(out)
        } else {
            Err(String::from_utf8_lossy(crate::util::name_bytes(&log)).into_owned())
        }
    }

    /// Restores `YAKOGL_SHADERC_TOKEN` to unset on drop, including on a
    /// failing assertion, so one test's env change cannot leak into another.
    struct TokenEnvGuard;
    impl Drop for TokenEnvGuard {
        fn drop(&mut self) {
            // SAFETY: the caller holds `serial()`, so no other test in this
            // binary reads or writes YAKOGL_SHADERC_TOKEN at the same time.
            unsafe { std::env::remove_var("YAKOGL_SHADERC_TOKEN") };
        }
    }

    /// Part 1 of the small-followups review: the client adds the `"token"`
    /// member `tools/shaderc_server.py` (`TOKEN_FIELD`) reads, but only when
    /// its own `YAKOGL_SHADERC_TOKEN` is set.
    #[test]
    fn token_env_set_adds_the_token_field() {
        let _serial = serial();
        fake_server();
        // SAFETY: guarded by `_serial` above.
        unsafe { std::env::set_var("YAKOGL_SHADERC_TOKEN", "s3cret-test-token") };
        let _guard = TokenEnvGuard;
        let sh = compile(SHC_STAGE_FRAGMENT, "void main(){/*token_env_set_adds_the_token_field*/}")
            .expect("the fake server accepts this source");
        /* The compile made the caller a holder of a live record; let go, as
         * glDeleteShader would. Kept, the record stays live for the rest of
         * the process and every later test that counts live records sees
         * one too many -- which one depends on the order the suite's
         * threads take `serial()`. */
        release(&sh);
        assert!(
            FAKE_LAST_HAD_TOKEN.load(std::sync::atomic::Ordering::SeqCst),
            "YAKOGL_SHADERC_TOKEN was set but the fake server saw no \"token\" field"
        );
    }

    /// The other half: unset, the request is exactly what it was before this
    /// field existed -- no `"token"` member appears.
    #[test]
    fn token_env_unset_sends_the_request_unchanged() {
        let _serial = serial();
        fake_server();
        // SAFETY: guarded by `_serial` above; also covers a prior test that
        // left the variable set had its guard not already cleared it.
        unsafe { std::env::remove_var("YAKOGL_SHADERC_TOKEN") };
        let sh = compile(SHC_STAGE_FRAGMENT, "void main(){/*token_env_unset_sends_the_request_unchanged*/}")
            .expect("the fake server accepts this source");
        /* The compile made the caller a holder of a live record; let go, as
         * glDeleteShader would. Kept, the record stays live for the rest of
         * the process and every later test that counts live records sees
         * one too many -- which one depends on the order the suite's
         * threads take `serial()`. */
        release(&sh);
        assert!(
            !FAKE_LAST_HAD_TOKEN.load(std::sync::atomic::Ordering::SeqCst),
            "YAKOGL_SHADERC_TOKEN was unset but the fake server saw a \"token\" field anyway"
        );
    }

    #[test]
    fn base64_decodes_the_reference_vectors() {
        assert_eq!(b64_decode("").unwrap(), b"");
        assert_eq!(b64_decode("Zg==").unwrap(), b"f");
        assert_eq!(b64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(b64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(b64_decode("+/+/").unwrap(), [0xfb, 0xff, 0xbf]);
        assert!(b64_decode("Zm9v*").is_none(), "a stray character is a bad reply, not a zero byte");
    }

    /* The two replies below are PowerVR-shaped. On a `gpu-vivante` build the
     * decode refuses them on purpose -- a reply with no `viv` section is the
     * server answering for the wrong GPU -- so they belong to that build, and
     * the GC7000 arm has its own set at the end of this module. */

    /// The server sends `blend_consts_off` only when the shader has blend
    /// constants. Absent must mean the sentinel, not dword 0: defaulting it to
    /// 0 made the draw path write glBlendColor over the first uniform of every
    /// fragment block, which on the i.MX8MP turned six gl_test cases black
    /// while their geometry stayed correct.
    #[test]
    fn an_absent_blend_consts_off_is_the_sentinel_not_dword_zero() {
        let mut rec = blank();
        let r = reply(
            r#"{"ok":true,"usc_dw":1,"blob":"AQAAAA==","reflection":{
                 "temps":2,"uniform_dwords":4,
                 "uniforms":[{"name":"u_color","dword_off":0,"dwords":4,"gl_type":35666,
                              "array_len":1,"binding":0,"inner":1}],
                 "const_contents":[2,2,2,2],"const_data":[0,1,2,3],
                 "viv":{"in":[],"out":[],"vs_pos_out_reg":-1,"vs_pointsize_out_reg":-1,
                        "vs_id_in_reg":-1,"ps_color_out_reg":1,"ps_depth_out_reg":-1,
                        "vs_load_balancing":0,"input_count_unk8":31}}}"#,
        );
        fill_shader(&r, &mut rec).unwrap();
        assert_eq!(
            rec.shader.blend_consts_off,
            super::super::SHC_NO_BLEND_CONSTS,
            "an omitted blend_consts_off means the shader has none"
        );
    }

    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn a_fragment_reply_fills_every_field() {
        let mut rec = blank();
        let r = reply(
            r#"{"ok":true,"usc_dw":2,"blob":"AQAAAAIAAAA=","reflection":{
                 "temps":8,"uses_w":1,"uses_discard":1,"meta_off":65535,
                 "fs_front_face_off1":3,"num_outputs":1,"num_tile_buffers":2,
                 "tile_buffer_sh":40,"uniform_dwords":8,"uniform_sh_dwords":4,
                 "blend_key":291,"uses_fbfetch":1,"blend_consts_off":255,
                 "varyings":[{"location":0,"components":4,"interp":2,"vtxout_off":4,"name":"v_uv"}],
                 "uniforms":[{"name":"uTex","dword_off":65535,"dwords":1,"gl_type":35678,
                              "array_len":1,"binding":1,"inner":1}]}}"#,
        );
        fill_shader(&r, &mut rec).unwrap();
        assert_eq!(rec.usc, [1, 2], "little-endian dwords");
        let sh = &rec.shader;
        assert_eq!((sh.temps, sh.uses_w, sh.uses_discard), (8, 1, 1));
        assert_eq!((sh.meta_off, sh.fs_front_face_off1), (0xffff, 3));
        assert_eq!((sh.num_outputs, sh.num_tile_buffers, sh.tile_buffer_sh), (1, 2, 40));
        assert_eq!((sh.uniform_dwords, sh.uniform_sh_dwords), (8, 4));
        assert_eq!((sh.blend_key, sh.uses_fbfetch, sh.blend_consts_off), (291, 1, 255));
        assert_eq!(sh.num_varyings, 1);
        assert_eq!((sh.varyings[0].components, sh.varyings[0].interp, sh.varyings[0].vtxout_off), (4, 2, 4));
        assert_eq!(name_bytes(&sh.varyings[0].name), b"v_uv");
        assert_eq!(rec.uniforms.len(), 1);
        assert_eq!(name_bytes(&rec.uniforms[0].name), b"uTex");
        assert_eq!((rec.uniforms[0].dword_off, rec.uniforms[0].binding), (0xffff, 1));
    }

    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn a_vertex_reply_fills_the_attributes() {
        let mut rec = blank();
        let r = reply(
            r#"{"ok":true,"usc_dw":1,"blob":"////AA==","reflection":{
                 "vs_output_size":12,"vs_psize_off":255,"vs_vertex_id_reg1":2,"vs_instance_id_reg1":3,
                 "attribs":[{"location":1,"components":3,"vi_reg":4,"format":0,"name":"aPos","gl_type":35665}]}}"#,
        );
        fill_shader(&r, &mut rec).unwrap();
        assert_eq!(rec.usc, [0x00ff_ffff]);
        let sh = &rec.shader;
        assert_eq!((sh.vs_output_size, sh.vs_psize_off), (12, 255));
        assert_eq!((sh.vs_vertex_id_reg1, sh.vs_instance_id_reg1), (2, 3));
        assert_eq!(sh.num_attribs, 1);
        assert_eq!((sh.attribs[0].location, sh.attribs[0].components, sh.attribs[0].vi_reg), (1, 3, 4));
        assert_eq!(sh.attribs[0].gl_type, 35665);
        assert_eq!(name_bytes(&sh.attribs[0].name), b"aPos");
    }

    #[test]
    fn a_reply_that_does_not_add_up_is_refused() {
        let mut rec = blank();
        let no_refl = reply(r#"{"ok":true,"usc_dw":1,"blob":"AQAAAA=="}"#);
        assert_eq!(fill_shader(&no_refl, &mut rec).unwrap_err(), "reply has no reflection");

        let no_blob = reply(r#"{"ok":true,"usc_dw":1,"reflection":{}}"#);
        assert_eq!(fill_shader(&no_blob, &mut rec).unwrap_err(), "reply has no USC binary");

        /* the announced dword count and the payload must agree */
        let short = reply(r#"{"ok":true,"usc_dw":2,"blob":"AQAAAA==","reflection":{}}"#);
        assert!(fill_shader(&short, &mut rec).unwrap_err().contains("not the 2 dwords announced"));

        let bad64 = reply(r#"{"ok":true,"usc_dw":1,"blob":"AQAA*A==","reflection":{}}"#);
        assert_eq!(fill_shader(&bad64, &mut rec).unwrap_err(), "the USC binary is not base64");
    }

    #[test]
    fn more_varyings_than_the_abi_holds_is_refused_not_truncated() {
        let mut rec = blank();
        let many: String = (0..17)
            .map(|i| format!(r#"{{"location":{i},"components":1,"vtxout_off":4}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let r = reply(&format!(r#"{{"ok":true,"usc_dw":1,"blob":"AQAAAA==","reflection":{{"varyings":[{many}]}}}}"#));
        assert!(fill_shader(&r, &mut rec).unwrap_err().contains("17 varyings, shc_shader holds 16"));
    }

    #[test]
    fn a_published_record_owns_what_its_shader_points_at() {
        let _serial = serial();
        let mut rec = blank();
        rec.sha = [7; 32];
        rec.usc = std::vec![0xdead_beef, 1];
        rec.uniforms = std::vec![ShcUniform::ZERO];
        let sh = admit_for_test(rec);
        assert_eq!(sh.usc_dw, 2);
        assert_eq!(sh.usc_words(), [0xdead_beef, 1]);
        assert_eq!(sh.num_uniforms, 1);
        assert_eq!(holders(&sh), Some(1), "priv is the live record, and the caller holds it");
        /* a lookup of the same key is a second holder of the same record */
        let Some(Answer::Shader(again)) = cache().find(&[7; 32], SHC_STAGE_FRAGMENT, 0) else {
            panic!("the live record was not found");
        };
        assert_eq!(again.priv_, sh.priv_);
        assert_eq!(holders(&sh), Some(2));
        release(&again);
        release(&sh);
        assert_eq!(holders(&sh), None, "the last release takes it out of the live list");
        assert!(cache().kept.iter().any(|r| r.sha == [7; 32]), "and keeps it for a recompile");
        cache_clear();
        assert!(cache().find(&[7; 32], SHC_STAGE_FRAGMENT, 0).is_none());
    }

    #[test]
    fn a_broken_host_is_not_a_broken_shader() {
        /* the whole point of the distinction: "ok" present means the reply is
         * about the shader, absent means it is about the host */
        let refused = reply(r#"{"ok":false,"error":"ERROR: 0:3: 'x' : undeclared identifier"}"#);
        assert!(matches!(refusal(&refused), Verdict::ShaderRefused(ref l) if l.contains("undeclared")));

        /* the pipeline side may call the log "log" instead of "error" */
        let refused_log = reply(r#"{"ok":false,"log":"ERROR: 0:3: syntax error","phase":"validate"}"#);
        assert!(matches!(refusal(&refused_log), Verdict::ShaderRefused(ref l) if l.contains("syntax error")));

        /* no verdict at all: the host says it could not compile */
        let broken = reply(r#"{"error":"cannot run yakogl-tools: No such file or directory"}"#);
        assert!(matches!(refusal(&broken), Verdict::ServiceBroken(ref w) if w.contains("cannot run")));

        /* a refusal with no text still says something useful */
        assert!(matches!(refusal(&reply(r#"{"ok":false}"#)), Verdict::ShaderRefused(ref l) if l.contains("refused")));
        assert!(matches!(refusal(&reply(r#"{}"#)), Verdict::ServiceBroken(ref w) if w.contains("no verdict")));

        assert!(matches!(refusal(&reply(r#"{"ok":true,"usc_dw":1}"#)), Verdict::Compiled));
    }

    #[test]
    fn a_shader_from_another_back_end_has_no_record() {
        let _serial = serial();
        assert!(cache().live_of(&ShcShader::ZERO).is_none());
        let mut out = ShcShader::ZERO;
        // SAFETY: valid pointers; the shader simply carries no remote record.
        assert_eq!(unsafe { remote_blend_variant(&ShcShader::ZERO, 1, &mut out) }, -EINVAL);
    }

    #[test]
    fn blend_variant_zero_copies_the_input_even_in_place() {
        let _serial = serial();
        let mut rec = blank();
        rec.sha = [9; 32];
        rec.usc = std::vec![42];
        let mut fs = admit_for_test(rec);
        fs.temps = 0x1234; /* marks the caller's copy */
        let p: *mut ShcShader = &mut fs;
        // SAFETY: in == out is allowed (the input is read by value first).
        assert_eq!(unsafe { remote_blend_variant(p, 0, p) }, 0);
        assert_eq!(fs.temps, 0x1234);
        assert_eq!(fs.usc_words(), [42]);
        /* the draw path releases every variant it holds, key 0 included */
        assert_eq!(holders(&fs), Some(2), "the variant is a copy the caller holds");
        release(&fs);
        release(&fs);
        assert_eq!(holders(&fs), None);
        cache_clear();
    }

    /* ---- ownership: what a delete frees, and what the cache keeps ---- */

    /// glCompileShader then glDeleteShader, N times over distinct sources
    /// (the release is what `destroy_shader` calls): nothing stays live, the
    /// kept list stops at its cap, and the most recent sources recompile
    /// without a round trip. Before the fix every record was leaked and the
    /// lookup list grew by one per compile, for the life of the process.
    #[test]
    fn compile_and_delete_frees_every_record_and_keeps_a_bounded_few() {
        let _serial = serial();
        cache_clear();
        let asked = fake_server();
        let cap = kept_cap();
        let n = cap + 36;
        let text = |i: usize| std::format!("void main() {{ gl_FragColor = vec4({i}.0); }}\n");
        for i in 0..n {
            let sh = compile(SHC_STAGE_FRAGMENT, &text(i)).unwrap();
            assert_eq!(cache_stats().live, 1, "compile {i}: the shader object holds it");
            release(&sh);
            let st = cache_stats();
            assert_eq!((st.live, st.kept), (0, (i + 1).min(cap)), "delete {i}: {st}");
        }
        assert_eq!(fake_server() - asked, n, "one round trip per distinct source");
        /* the newest source is kept: served locally, and live again */
        let sh = compile(SHC_STAGE_FRAGMENT, &text(n - 1)).unwrap();
        assert_eq!(fake_server() - asked, n, "a kept record is a cache hit");
        assert_eq!((cache_stats().live, cache_stats().kept), (1, cap - 1));
        release(&sh);
        /* the oldest was evicted: asked again */
        release(&compile(SHC_STAGE_FRAGMENT, &text(0)).unwrap());
        assert_eq!(fake_server() - asked, n + 1, "an evicted record is fetched again");
        assert_eq!(cache_stats().live, 0);
        cache_clear();
    }

    /// A refused shader is handed to no one, so it is never live; it is kept
    /// with its log, and the same broken text is not sent twice.
    #[test]
    fn a_refused_shader_is_kept_not_held() {
        let _serial = serial();
        cache_clear();
        let asked = fake_server();
        for _ in 0..3 {
            let Err(log) = compile(SHC_STAGE_VERTEX, "BROKEN") else {
                panic!("a refused shader compiled");
            };
            assert!(log.contains("BROKEN is not GLSL"), "{log}");
        }
        assert_eq!(fake_server() - asked, 1, "asked once");
        assert_eq!((cache_stats().live, cache_stats().kept), (0, 1));
        cache_clear();
    }

    /// The link verdicts are bounded like the records, but a pair whose two
    /// stages are live (a linked program holds both) is never evicted: its
    /// relink must stay free. Once its stages go, it is evicted like any.
    #[test]
    fn the_link_cache_never_evicts_a_pair_in_use() {
        let _serial = serial();
        cache_clear();
        let cap = kept_cap();
        let stage = |sha: u8, st: u32| {
            let mut rec = blank();
            rec.sha = [sha; 32];
            rec.stage = st;
            rec.usc = std::vec![1];
            admit_for_test(rec)
        };
        let (vs, fs) = (stage(0xa1, SHC_STAGE_VERTEX), stage(0xa2, SHC_STAGE_FRAGMENT));
        let unused = |i: usize| {
            let mut sha = [0u8; 32];
            sha[..8].copy_from_slice(&(i as u64).to_le_bytes());
            Linked { vs: sha, fs: sha, error: None }
        };
        let mut c = cache();
        c.linked_put(Linked { vs: [0xa1; 32], fs: [0xa2; 32], error: None });
        /* the pair in use is the oldest, the one a plain LRU drops first;
         * push cap + 10 unused pairs past it */
        for i in 0..cap + 10 {
            c.linked_put(unused(i));
        }
        assert_eq!(c.linked.len(), cap, "unused pairs were evicted in its place");
        assert!(c.linked.iter().any(|l| l.vs == [0xa1; 32]), "the pair in use survived as the oldest");
        drop(c);
        /* glDeleteProgram and glDeleteShader: the pair is no longer in use */
        release(&vs);
        release(&fs);
        let mut c = cache();
        c.linked_put(unused(cap + 10));
        assert_eq!(c.linked.len(), cap);
        assert!(!c.linked.iter().any(|l| l.vs == [0xa1; 32]), "unused now, it went first");
        drop(c);
        cache_clear();
    }

    /// The high-water line: the peak does not come down when the records go,
    /// and the line names every figure.
    #[test]
    fn the_high_water_line_keeps_its_peak() {
        let _serial = serial();
        cache_clear();
        let before = cache_stats();
        let held: Vec<ShcShader> = (0..3u8)
            .map(|i| {
                let mut rec = blank();
                rec.sha = [0xc0 + i; 32];
                rec.usc = std::vec![1];
                admit_for_test(rec)
            })
            .collect();
        let peak = cache_stats().live_peak;
        assert!(peak >= before.live + 3);
        for sh in &held {
            release(sh);
        }
        let st = cache_stats();
        assert_eq!((st.live, st.live_peak), (before.live, peak), "{st}");
        let line = std::format!("{st}");
        assert_eq!(
            line,
            std::format!(
                "remote shaderc: {} live (peak {peak}), {} kept of {}, {} link verdicts (peak {})",
                st.live,
                st.kept,
                st.kept_cap,
                st.linked,
                st.linked_peak
            )
        );
        std::println!("{line}");
        assert_eq!(shc_remote_cache_report(), 0);
        cache_clear();
    }

    /// Through the GL entry points an application calls: a program keeps the
    /// shaders it was linked from alive after glDeleteShader, and the last
    /// glDeleteProgram frees them (they move to the kept list).
    #[cfg(feature = "host-test")]
    #[test]
    fn a_program_keeps_its_shaders_until_it_is_deleted() {
        use crate::gl::consts::{GL_COMPILE_STATUS, GL_FRAGMENT_SHADER, GL_LINK_STATUS, GL_VERTEX_SHADER};
        use crate::gl::program::{
            glAttachShader, glCreateProgram, glDeleteProgram, glDetachShader, glGetProgramiv, glLinkProgram,
        };
        use crate::gl::shader::{glCompileShader, glCreateShader, glDeleteShader, glGetShaderiv, glShaderSource};

        /* Restores the back end INCLUDING through a panic. */
        struct BackEnd(*const ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { super::super::g_shaderc = self.0 };
            }
        }

        let _serial = serial();
        cache_clear();
        let asked = fake_server();
        let _gl = crate::testutil::TestGl::new(16, 16);
        // SAFETY: TestGl serialises the GL suite; the restorer puts it back.
        let _be = BackEnd(unsafe { super::super::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { super::super::g_shaderc = &SHC_REMOTE };

        let shader = |ty: u32, text: &str| {
            let text = std::ffi::CString::new(text).unwrap();
            // SAFETY: plain GL calls on a handle this test owns.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok = 0;
                glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "{text:?} did not compile");
                s
            }
        };
        let vs_text = "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
        let fs_text = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
        let (v, f) = (shader(GL_VERTEX_SHADER, vs_text), shader(GL_FRAGMENT_SHADER, fs_text));
        assert_eq!(cache_stats().live, 2);
        // SAFETY: plain GL calls on handles this test owns.
        let p = unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "the program did not link");
            /* Deleting an attached shader only flags it (ES 2.0 2.10.3);
             * detached, the shader objects really go. */
            glDeleteShader(v);
            glDeleteShader(f);
            glDetachShader(p, v);
            glDetachShader(p, f);
            p
        };
        assert_eq!(cache_stats().live, 2, "the program's snapshot holds both stages");
        // SAFETY: as above.
        unsafe { glDeleteProgram(p) };
        let st = cache_stats();
        assert_eq!((st.live, st.kept), (0, 2), "the last owner is gone: {st}");
        /* and a recompile of the same text is served from the kept list */
        let v2 = shader(GL_VERTEX_SHADER, vs_text);
        assert_eq!(fake_server() - asked, 2, "two compiles in all, the recompile included");
        // SAFETY: as above.
        unsafe { glDeleteShader(v2) };
        assert_eq!(cache_stats().live, 0);
        cache_clear();
    }

    /* ---- the GC7000 arm ----
     *
     * `parse_viv` had no unit test at all: `host-test,gpu-vivante` has no
     * remote back end compiled in and `host-test,shaderc-remote` has no
     * GC7000, so the pair that has both was never built -- and did not
     * compile, because `blank()` was missing the two `gpu-vivante` fields.
     * The register wiring these decode is what the comment on `parse_viv`
     * says cost a day: a shader with correct code and no wiring draws
     * nothing recognisable and looks exactly like a code-generation bug. */

    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_gc7000_reply_fills_the_register_wiring() {
        use crate::shaderc::SHC_VIV_NO_REG;
        let mut rec = blank();
        let r = reply(
            r#"{"ok":true,"usc_dw":1,"blob":"AQAAAA==","reflection":{
                 "temps":4,
                 "viv":{"in":[{"reg":1,"slot":32,"components":2,"interp":0,"name":"v_uv"}],
                        "out":[{"reg":0,"slot":0,"components":4,"interp":0,"name":"gl_FragColor"}],
                        "vs_pos_out_reg":-1,"vs_pointsize_out_reg":-1,"vs_id_in_reg":-1,
                        "ps_color_out_reg":0,"ps_depth_out_reg":-1,
                        "vs_load_balancing":42,"input_count_unk8":7},
                 "const_contents":[1,2],"const_data":[16,32]}}"#,
        );
        fill_shader(&r, &mut rec).unwrap();
        let viv = &rec.shader.viv;
        assert_eq!((viv.num_in, viv.num_out), (1, 1));
        assert_eq!((viv.in_[0].reg, viv.in_[0].slot, viv.in_[0].components), (1, 32, 2));
        assert_eq!(name_bytes(&viv.in_[0].name), b"v_uv");
        assert_eq!(name_bytes(&viv.out[0].name), b"gl_FragColor");
        assert_eq!(viv.ps_color_out_reg, 0);
        assert_eq!(
            (viv.vs_pos_out_reg, viv.vs_pointsize_out_reg, viv.vs_id_in_reg, viv.ps_depth_out_reg),
            (SHC_VIV_NO_REG, SHC_VIV_NO_REG, SHC_VIV_NO_REG, SHC_VIV_NO_REG),
            "-1 on the wire is 'this shader has none', not register 255"
        );
        assert_eq!((viv.vs_load_balancing, viv.input_count_unk8), (42, 7));
        assert_eq!(rec.const_contents.len(), 2);
        assert_eq!(rec.const_data, [16, 32]);
    }

    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_powervr_shaped_reply_says_the_server_answered_for_the_wrong_gpu() {
        /* The failure this names is a server compiling for the other GPU. It
         * has to be said plainly: the alternative is a shader with no wiring,
         * which renders nothing and looks like a compiler bug. */
        let mut rec = blank();
        let r = reply(r#"{"ok":true,"usc_dw":1,"blob":"AQAAAA==","reflection":{"temps":4}}"#);
        let err = fill_shader(&r, &mut rec).unwrap_err();
        assert!(err.contains("no GC7000 reflection"), "{err}");
    }

    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn parallel_constant_arrays_of_different_lengths_are_refused() {
        let mut rec = blank();
        let r = reply(
            r#"{"ok":true,"usc_dw":1,"blob":"AQAAAA==","reflection":{
                 "viv":{"in":[],"out":[]},"const_contents":[1,2,3],"const_data":[16]}}"#,
        );
        let err = fill_shader(&r, &mut rec).unwrap_err();
        assert!(err.contains("const_contents has 3 entries, const_data 1"), "{err}");
    }

    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn more_io_registers_than_the_struct_holds_are_refused_not_dropped() {
        use crate::shaderc::SHC_VIV_MAX_IO;
        let mut rec = blank();
        let one = r#"{"reg":1,"slot":0,"components":4,"interp":0,"name":"v"}"#;
        let too_many = std::vec![one; SHC_VIV_MAX_IO + 1].join(",");
        let r = reply(&std::format!(
            r#"{{"ok":true,"usc_dw":1,"blob":"AQAAAA==","reflection":{{
                 "viv":{{"in":[{too_many}],"out":[]}}}}}}"#
        ));
        let err = fill_shader(&r, &mut rec).unwrap_err();
        assert!(err.contains("in registers, shc_viv holds"), "{err}");
    }
}
