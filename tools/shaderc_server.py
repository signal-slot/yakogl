#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# shaderc_server.py - compile GLSL ES on this host for a YakoGL client.
#
# The offline shader backend (src/shaderc/offline.rs) can only run shaders that
# were compiled into the blob table at build time. The Khronos dEQP-GLES2 suite
# makes its shaders up as it runs, so nothing can be precompiled. There is no
# compiler on the board, so the board asks this server instead: the remote
# shader backend (src/shaderc/remote.rs) sends the GLSL text over TCP and gets
# back the shader binary plus exactly the reflection the blob table would hold.
#
# Nothing about a toolchain is duplicated here. This file is transport, cache
# and routing; the compile is the same pipeline that writes the offline table:
#
#   gpu=pvr       `yakogl-tools shaderc-compile` (tools/yakogl-tools), which
#                 calls glsl2usc's resolve_shader() -> glslang -> pco_shc and
#                 builds the reply from the table emitter, so a remote shader
#                 and a table row are identical dword for dword.
#   gpu=vivante   tools/vivante/gen_shader_table.py's reflect(), which is
#                 glsl2vivante.py's translation -> glslang -> etna_shc.
#
# Protocol (both directions): a 4-byte big-endian byte count, then that many
# bytes of UTF-8 JSON. One request per connection.
#
#   compile  {"gpu": "pvr",                  # "vivante" for the GC7000
#             "op": "compile",               # the default when absent
#             "stage": "vert"|"frag",        # "vs"/"fs" also accepted
#             "source": "<GLSL ES 1.00 / 3.00 or GLSL 450 text>",
#             "blend_key": <uint32>,         # 0 / absent = the plain shader
#             "blend": "<glsl2usc --blend spec>",   # instead of blend_key
#             "colormask": "rgba subset"}    # with "blend", instead of ":MASK"
#
#        ->  {"ok": true, "blob": "<base64 shader binary, little-endian>",
#             "usc_dw": <n>, "reflection": {...}, "cached": true|false}
#        ->  {"ok": false, "error": "<compiler log>"}   the SHADER is bad
#        ->  {"error": "<what broke>"}                  the SERVICE is broken
#
# The last two must not be confused, and the difference is whether "ok" is
# present at all. A shader the compiler rejected is the application's business:
# the client caches it and reports GL_COMPILE_STATUS false with that log, which
# is what dEQP-GLES2's negative cases read. A host that cannot compile at all
# (no pco_shc, the tool missing, a timeout, a reply that is not JSON) is not the
# application's business: the client must not cache it and must not tell the
# application its shader is bad.
#
#   link     {"gpu": "pvr", "op": "link",
#             "vert": "<VS GLSL>", "frag": "<FS GLSL>",
#             "max_varying_vectors": <n>}   # optional: the client's limit
#        ->  {"ok": true}  |  {"ok": false, "error": "<ES link log>"}
#             (and the same {"error": ...} for a broken service)
#
# The link request exists because the compiled shaders no longer carry what the
# ES link rules are about: `mediump vec3` and `highp vec3` compile to the same
# thing. See tools/yakogl-tools/src/link.rs.
#
# Usage:
#   tools/shaderc_server.py                        # 127.0.0.1:7777, this host only
#   tools/shaderc_server.py --port 7777 --cache ~/.cache/yakogl-shaderc
#   YAKOGL_SHADERC_TOKEN=<secret> tools/shaderc_server.py --host 0.0.0.0
#   tools/shaderc_server.py --host 0.0.0.0 --no-auth    # a board on the LAN, no token
#   tools/shaderc_server.py --client --stage frag some.frag   # one-shot probe
#
# Exposure and limits. Anyone who can connect can make this host run glslang
# and pco_shc, so:
#
#   bind          127.0.0.1 by default. A board reaches the server over the
#                 LAN, so it needs `--host 0.0.0.0` (or the host's address);
#                 a non-loopback bind prints a one-line warning.
#   token         YAKOGL_SHADERC_TOKEN on the server. When it is set, every
#                 request must carry the same string in its "token" member,
#                 or it gets a service error before any cache lookup or
#                 compile. The member is not part of the cache key and is
#                 removed before the request reaches a pipeline. When it is
#                 unset, a non-loopback bind refuses to start unless
#                 `--no-auth` says the exposure is on purpose.
#   --jobs        compiler runs at once (default: the CPU count); a request
#                 over the limit waits for a slot.
#   --max-connections
#                 open connections, one thread each (default 64); one more is
#                 answered with a service error and closed.
#   MAX_REQUEST_BYTES   16 MiB, the largest request frame accepted.
#   COMPILE_TIMEOUT_S   60 s, the wall-clock limit of one external compiler
#                 run (`--timeout`); a socket read or write gets 30 s more.
#   --cache-max-mb
#                 the size of the --cache directory's own entries (default
#                 512; 0 = no limit). Past it, the least recently used entries
#                 (file mtime; a hit refreshes it) are deleted, except those
#                 this process wrote in the last CACHE_PROTECT_S seconds.
#
# Every refusal is a service error with its own fixed message (the REFUSE_*
# constants), so a measurement can group by it.
#
# Config: PCO_SHC / GLSLANG as for the offline table; ETNA_SHC for gpu=vivante.
# YAKOGL_TOOLS points at a prebuilt yakogl-tools binary (else one is built with
# cargo at startup). GLSL2USC_CACHE (the compiler's own raw-output cache) is
# pointed inside --cache unless it is already set.
import argparse
import base64
import datetime
import hashlib
import hmac
import ipaddress
import json
import os
import socket
import socketserver
import struct
import subprocess
import sys
import threading
import time

_HERE = os.path.dirname(os.path.abspath(__file__))
_ROOT = os.path.dirname(_HERE)

PROTO_VERSION = 2           # 1 was the C server: compile only, PowerVR only
MAX_REQUEST_BYTES = 16 << 20    # a shader request is a few KB; cap the framing
DEFAULT_PORT = 7777
COMPILE_TIMEOUT_S = 60      # wall-clock seconds for one glslang/pco_shc run
IO_GRACE_S = 30             # a socket read or write may take this much longer
DEFAULT_MAX_CONNECTIONS = 64
DEFAULT_CACHE_MAX_MB = 512
CACHE_PROTECT_S = 10 * 60   # never evict what this process wrote this recently
TOKEN_ENV = "YAKOGL_SHADERC_TOKEN"
TOKEN_FIELD = "token"       # the request member that carries it

# The refusals. Each is a fixed string so that a log or a measurement can
# group by it; a detail, when there is one, goes after the colon.
REFUSE_TOKEN_MISSING = "refused: this server needs a token and the request has none"
REFUSE_TOKEN_WRONG = "refused: the request's token does not match this server's"
REFUSE_CONNECTIONS = "refused: too many connections"
REFUSE_TOO_LARGE = "refused: the request is larger than the server accepts"
REFUSALS = (REFUSE_TOKEN_MISSING, REFUSE_TOKEN_WRONG, REFUSE_CONNECTIONS, REFUSE_TOO_LARGE)


class RequestTooLarge(ValueError):
    pass

STAGES = {"vert": "vert", "vs": "vert", "vertex": "vert",
          "frag": "frag", "fs": "frag", "fragment": "frag"}


# ---- framing ---------------------------------------------------------------

def send_msg(sock, obj):
    body = json.dumps(obj).encode("utf-8")
    sock.sendall(struct.pack(">I", len(body)) + body)


def recv_exactly(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            return None
        buf += chunk
    return buf


def recv_msg(sock):
    hdr = recv_exactly(sock, 4)
    if hdr is None:
        return None
    (n,) = struct.unpack(">I", hdr)
    if n > MAX_REQUEST_BYTES:
        raise RequestTooLarge("message length %d over %d" % (n, MAX_REQUEST_BYTES))
    if n == 0:
        raise ValueError("message length 0")
    body = recv_exactly(sock, n)
    if body is None:
        return None
    return json.loads(body.decode("utf-8"))


# ---- the pipelines ---------------------------------------------------------

def _file_id(path):
    """Identify a tool by what it is, so a rebuilt compiler misses the cache."""
    try:
        st = os.stat(path)
        return "%d:%d" % (st.st_size, int(st.st_mtime))
    except OSError:
        return "?"


def pco_shc_path():
    return os.environ.get("PCO_SHC", "")


def service_error(what):
    """A failure that is not the shader's fault. No "ok" member: the client
    reads that as "the service is broken", reports it as an internal failure
    rather than a compile error, and does not remember it."""
    return {"error": what}


class Pipeline:
    """One GPU's compiler. `tool_id` goes into every cache key, so a rebuilt
    compiler or a changed generator never serves a stale result."""

    name = "?"
    tool_id = "?"

    def compile(self, req):
        raise NotImplementedError

    def link(self, req):
        return service_error("the %s pipeline has no link check" % self.name)


class PvrPipeline(Pipeline):
    """PowerVR BXS-4-64: `yakogl-tools shaderc-compile`, one process per
    request. The tool speaks this protocol's request and reply objects
    verbatim, so there is nothing to translate here; it is the pipeline that
    writes src/shaderc/blobs.rs, so what comes back is byte-identical to a
    table row."""

    name = "pvr"

    def __init__(self, tool, timeout):
        self.tool = tool
        self.timeout = timeout
        self.tool_id = "|".join(_file_id(f) for f in (tool, pco_shc_path()))

    def _run(self, req):
        try:
            r = subprocess.run([self.tool, "shaderc-compile"],
                               input=json.dumps(req), capture_output=True,
                               text=True, timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return service_error("the compiler timed out after %ss" % self.timeout)
        except OSError as e:
            return service_error("cannot run %s: %s" % (self.tool, e))
        if r.returncode != 0:
            # The tool exits non-zero for a malformed request or a broken
            # environment; a shader it refuses is a reply with ok:false.
            return service_error(r.stderr.strip() or "yakogl-tools exited %d" % r.returncode)
        try:
            return json.loads(r.stdout)
        except ValueError as e:
            return service_error("yakogl-tools wrote no reply (%s): %s" % (e, r.stdout[:200]))

    def compile(self, req):
        return self._run(req)

    def link(self, req):
        return self._run(req)


class VivantePipeline(Pipeline):
    """Vivante GC7000 (i.MX8MP): tools/vivante/gen_shader_table.py's reflect(),
    which is glsl2vivante.py's GLSL 4.60 rewrite -> glslang -G -> etna_shc.
    That function returns exactly the fields a GC7000 table row holds, so the
    reflection here is the table's, as on the PowerVR side.

    Blending and colour masks are pixel-engine state on this part, so there
    are no blend variants: a non-zero blend key has nothing to compile."""

    name = "vivante"

    def __init__(self, timeout, tool=""):
        # The generator runs the compilers itself; `timeout` and `tool` are
        # here for the link check, which shells out to the same host tool the
        # PowerVR pipeline uses (see `link` below).
        self.timeout = timeout
        self.tool = tool
        sys.path.insert(0, os.path.join(_HERE, "vivante"))
        import gen_shader_table         # noqa: E402  (needs the path above)
        self.gen = gen_shader_table
        self.tool_id = "|".join(_file_id(f) for f in
                                (os.environ.get("ETNA_SHC", ""), gen_shader_table.__file__, tool))

    def compile(self, req):
        if int(req.get("blend_key", 0) or 0) != 0:
            return service_error("the GC7000 blends in the pixel engine: no blend variants")
        stage = STAGES[req["stage"]]
        path = "shader.vert" if stage == "vert" else "shader.frag"
        try:
            sh = self.gen.reflect(path, text=req["source"])
        except self.gen.Fail as e:
            return {"ok": False, "error": str(e)}    # the shader: Fail is a diagnosed refusal
        except Exception as e:                       # noqa: BLE001 - keep serving
            return service_error("%s: %s" % (type(e).__name__, e))
        code = sh.pop("code")
        raw = b"".join(struct.pack("<I", w & 0xffffffff) for w in code)
        for drop in ("text", "sha256", "name"):
            sh.pop(drop, None)
        return {"ok": True, "blob": base64.b64encode(raw).decode("ascii"),
                "usc_dw": len(code), "origin": "etna_shc", "reflection": sh}

    def link(self, req):
        """Answer a link request for the GC7000.

        Two checks, and they are about different things.

        The ES link rules are the GLSL's, not the machine's -- a varying the
        fragment shader reads and the vertex shader never declares is a link
        error on every GPU there is -- so they run through the same host tool
        the PowerVR pipeline uses, rather than being written a second time
        here. `yakogl-tools shaderc-compile`'s link operation never looks at
        `gpu` for exactly that reason. Two implementations of one
        specification are free to drift, and the one that drifts is the one
        nobody runs.

        What IS this GPU's is the wiring: the GC7000 pairs the stages by
        REGISTER NUMBER, not by name (docs/online-shaderc/gc7000-abi.md), so a
        pair that satisfies the ES rules can still be unprogrammable here --
        a fragment input with no vertex output to feed it, or one whose
        register is past the end of what the vertex stage writes. That check
        needs both stages compiled, so it runs second and only once the source
        rules have passed.

        A missing or broken compiler is a service error at every step: it is
        never a verdict on the shader.
        """
        es = self._es_link_rules(req)
        if es is not None:
            return es

        wiring = self._wiring(req)
        if wiring is not None:
            return wiring
        return {"ok": True}

    def _es_link_rules(self, req):
        """The GLSL ES link rules, through the host tool. Returns None when
        they pass, or the reply to send."""
        if not self.tool:
            return service_error("no yakogl-tools on this host: the ES link "
                                 "rules cannot be checked")
        try:
            link = {"op": "link", "vert": req.get("vert", ""), "frag": req.get("frag", "")}
            # the client GPU's GL_MAX_VARYING_VECTORS (15 on the GC7000);
            # the tool's own default is the PowerVR's
            if "max_varying_vectors" in req:
                link["max_varying_vectors"] = req["max_varying_vectors"]
            r = subprocess.run([self.tool, "shaderc-compile"],
                               input=json.dumps(link),
                               capture_output=True, text=True, timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return service_error("the link check timed out after %ss" % self.timeout)
        except OSError as e:
            return service_error("cannot run %s: %s" % (self.tool, e))
        if r.returncode != 0:
            return service_error(r.stderr.strip() or "yakogl-tools exited %d" % r.returncode)
        try:
            reply = json.loads(r.stdout)
        except ValueError as e:
            return service_error("yakogl-tools wrote no reply (%s): %s" % (e, r.stdout[:200]))
        if "ok" not in reply:
            return reply                            # already a service error
        return None if reply["ok"] else reply       # a real link log

    def _wiring(self, req):
        """The GC7000's own rule: every fragment input must be fed by a vertex
        output of the same name, in a register the vertex stage actually
        writes. Returns None when it holds, or the reply to send."""
        try:
            vs = self.gen.reflect("shader.vert", text=req.get("vert", ""))
            fs = self.gen.reflect("shader.frag", text=req.get("frag", ""))
        except self.gen.Fail as e:
            # The ES rules above already accepted this pair, so a stage that
            # will not compile HERE is this pipeline failing to reproduce a
            # compile, not a verdict on the program. Calling it a link failure
            # would void a program whose stages the client compiled fine --
            # dEQP compiles both stages before it links, so by this point they
            # are known to compile. Report it as the service problem it is.
            return service_error("the GC7000 compiler cannot rebuild a stage "
                                 "the ES link rules accepted: %s" % e)
        except Exception as e:                       # noqa: BLE001 - keep serving
            return service_error("%s: %s" % (type(e).__name__, e))

        outs = {e["name"]: e for e in vs.get("out", [])}
        # The highest register the vertex stage writes, so a fragment input
        # past it is caught as such rather than as a missing name.
        top = max([e["reg"] for e in vs.get("out", [])] +
                  [vs.get("vs_pos_out_reg", 0) or 0], default=0)
        log = []
        for e in fs.get("in", []):
            name = e["name"]
            src = outs.get(name)
            if src is None:
                log.append("varying '%s' is read by the fragment shader and "
                           "never written by the vertex shader" % name)
                continue
            if src["components"] < e["components"]:
                log.append("varying '%s': the fragment shader reads %d "
                           "components, the vertex shader writes %d"
                           % (name, e["components"], src["components"]))
            if src["reg"] > top:
                log.append("varying '%s': vertex output register %d is past "
                           "the last register the vertex stage writes (%d)"
                           % (name, src["reg"], top))
        if log:
            return {"ok": False, "error": "\n".join(log)}
        return None


def build_provenance(tool):
    """What the running server was built from, for a runner that has to record
    it per run: the repository commit, whether the tree was clean, and how the
    tool binary relates to that commit.

    The last part is the one that matters. A stale binary against a fresh
    checkout produces results from the old compiler with no outward sign, and a
    sweep measured that way is worse than no sweep -- it looks like data. So
    the tool's own mtime is compared with the commit's, and an older binary is
    called out rather than merely reported.
    """
    def git(*a):
        try:
            out = subprocess.run(["git", "-C", _ROOT] + list(a), capture_output=True,
                                 text=True, timeout=10)
            return out.stdout.strip() if out.returncode == 0 else ""
        except Exception:
            return ""

    commit = git("rev-parse", "--short=12", "HEAD") or "(not a git checkout)"
    dirty = bool(git("status", "--porcelain"))
    when = git("show", "-s", "--format=%ct", "HEAD")
    note = ""
    try:
        if when and os.path.exists(tool) and os.path.getmtime(tool) < int(when):
            note = "  **STALE: the binary predates the commit; rebuild yakogl-tools**"
    except Exception:
        pass
    return "%s%s%s" % (commit, " (working tree dirty)" if dirty else "", note)


def find_tool(explicit):
    """The yakogl-tools binary: --tool, then $YAKOGL_TOOLS, then a build tree,
    then cargo. Built once here rather than per request, because a request
    pays for glslang and pco_shc already."""
    if explicit:
        return os.path.abspath(explicit)
    env = os.environ.get("YAKOGL_TOOLS")
    if env:
        return os.path.abspath(env)
    for d in (".target", "target"):
        for prof in ("release", "debug"):
            p = os.path.join(_ROOT, d, prof, "yakogl-tools")
            if os.access(p, os.X_OK):
                return p
    log("building yakogl-tools (set YAKOGL_TOOLS to skip this)")
    subprocess.run(["cargo", "build", "--quiet", "--release",
                    "--manifest-path", os.path.join(_ROOT, "Cargo.toml"),
                    "-p", "yakogl-tools"], check=True)
    for d in (".target", "target"):
        p = os.path.join(_ROOT, d, "release", "yakogl-tools")
        if os.access(p, os.X_OK):
            return p
    raise SystemExit("shaderc_server: cargo built yakogl-tools but it is not where it was looked for")


# ---- the cache -------------------------------------------------------------

class Cache:
    """One JSON file per result, named by the key. A failure is cached too: an
    application that recompiles the same broken source must not make the whole
    suite pay for it twice.

    The directory is capped at `max_bytes` of its own entries (the *.json
    files at its top; the compiler's raw-output cache under pco/ is not
    counted). A hit refreshes the entry's mtime, so eviction by oldest mtime
    is least-recently-used. An entry this process wrote less than
    `protect_s` seconds ago is never evicted: a conformance run compiles a
    shader, links it, and asks again, and losing the answer in between would
    only cost time, but a cap small enough to do that is a misconfiguration
    that should show as an over-cap log line, not as churn."""

    SLACK = 0.9     # evict down to 90% of the cap, so a scan is not per put

    def __init__(self, directory, max_bytes=0, protect_s=CACHE_PROTECT_S):
        self.dir = directory
        self.max_bytes = max_bytes
        self.protect_s = protect_s
        self.lock = threading.Lock()
        self.written = {}        # name -> time.monotonic() of this process's put
        self.total = 0
        self.warned_full = False
        if directory:
            os.makedirs(directory, exist_ok=True)
            self.total = sum(size for _, _, size in self._entries())

    @staticmethod
    def key(pipeline, req):
        """Everything that decides the output: the protocol, the GPU, the
        operation, the compiler AND the pipeline driving it, then the
        request's own text."""
        h = hashlib.sha256()
        h.update(("yakogl-shaderc/%d|%s|%s|%s|%s|%s|" % (
            PROTO_VERSION, pipeline.name, req.get("op", "compile"),
            STAGES.get(str(req.get("stage", "")), ""),
            "%s:%s:%s" % (req.get("blend_key", 0), req.get("blend", ""), req.get("colormask", "")),
            pipeline.tool_id)).encode())
        for field in ("source", "vert", "frag"):
            h.update(("\0%s\0" % field).encode())
            h.update(str(req.get(field, "")).encode("utf-8", "replace"))
        return h.hexdigest()

    def _entries(self):
        """(name, mtime, size) of every entry file."""
        out = []
        try:
            with os.scandir(self.dir) as it:
                for e in it:
                    if not e.name.endswith(".json") or not e.is_file(follow_symlinks=False):
                        continue
                    try:
                        st = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    out.append((e.name, st.st_mtime, st.st_size))
        except OSError:
            pass
        return out

    def get(self, key):
        if not self.dir:
            return None
        path = os.path.join(self.dir, key + ".json")
        try:
            with open(path) as f:
                reply = json.load(f)
        except (OSError, ValueError):
            return None      # a missing or truncated cache file means recompiling
        try:
            os.utime(path)   # recently used: the eviction order is by mtime
        except OSError:
            pass
        return reply

    def put(self, key, reply):
        if not self.dir:
            return
        name = key + ".json"
        path = os.path.join(self.dir, name)
        tmp = "%s.%d.%d.tmp" % (path, os.getpid(), threading.get_ident())
        try:
            with open(tmp, "w") as f:
                json.dump(reply, f)
            size = os.path.getsize(tmp)
            try:
                old = os.path.getsize(path)
            except OSError:
                old = 0
            os.replace(tmp, path)
        except OSError:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            return
        with self.lock:
            self.written[name] = time.monotonic()
            self.total += size - old
            if self.max_bytes and self.total > self.max_bytes:
                self._evict()

    def _evict(self):
        """Delete the oldest entries until the directory is under SLACK of the
        cap. The total is recounted from the directory first, because another
        server or a person may share it. Called with the lock held."""
        now = time.monotonic()
        self.written = {n: t for n, t in self.written.items() if now - t < self.protect_s}
        entries = self._entries()
        self.total = sum(size for _, _, size in entries)
        goal = int(self.max_bytes * self.SLACK)
        for name, _, size in sorted(entries, key=lambda e: e[1]):
            if self.total <= goal:
                break
            if name in self.written:
                continue
            try:
                os.unlink(os.path.join(self.dir, name))
            except OSError:
                continue
            self.total -= size
        if self.total > self.max_bytes:
            if not self.warned_full:
                log("cache over its cap (%d > %d bytes): every remaining entry was "
                    "written in the last %ds" % (self.total, self.max_bytes, self.protect_s))
                self.warned_full = True
        else:
            self.warned_full = False


# ---- the server ------------------------------------------------------------

def log(msg):
    stamp = datetime.datetime.now().strftime("%H:%M:%S")
    sys.stdout.write("shaderc_server %s %s\n" % (stamp, msg))
    sys.stdout.flush()


def sha12(text):
    return hashlib.sha256(text.encode("utf-8", "replace")).hexdigest()[:12]


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        peer = "%s:%d" % self.client_address[:2]
        self.request.settimeout(self.server.compiler_timeout + IO_GRACE_S)
        try:
            req = recv_msg(self.request)
        except RequestTooLarge as e:
            log("%s %s (%s)" % (peer, REFUSE_TOO_LARGE, e))
            refuse(self.request, REFUSE_TOO_LARGE)
            return
        except (ValueError, OSError, socket.timeout) as e:
            log("%s bad request: %s" % (peer, e))
            return
        if req is None:
            return
        try:
            reply, cached, note = self.serve(req)
        except Exception as e:                       # noqa: BLE001 - keep serving
            reply = service_error("%s: %s" % (type(e).__name__, e))
            cached, note = False, "server error: %s" % e
        try:
            send_msg(self.request, reply)
        except OSError as e:
            log("%s reply failed: %s" % (peer, e))
            return
        log("%s %s" % (peer, note + (" [cache]" if cached else "")))

    def serve(self, req):
        if not isinstance(req, dict):
            return service_error("the request must be a JSON object"), False, "bad request"
        # The token first: nothing, not even a cache hit, for a caller
        # without it. Then it is dropped, so neither the cache key nor the
        # host tool (which reads the request verbatim) ever sees it.
        refusal = self.server.check_token(req.pop(TOKEN_FIELD, None))
        if refusal:
            return service_error(refusal), False, refusal
        gpu = str(req.get("gpu", "pvr"))
        pipeline = self.server.pipelines.get(gpu)
        if pipeline is None:
            known = ", ".join(sorted(self.server.pipelines))
            err = "unknown gpu %r (this server serves: %s)" % (gpu, known)
            return service_error(err), False, err
        op = str(req.get("op", "compile"))
        if op not in ("compile", "link"):
            return service_error("unknown op %r" % op), False, "unknown op %r" % op
        if op == "compile":
            stage = str(req.get("stage", ""))
            if stage not in STAGES:
                err = "stage must be vert or frag (got %r)" % stage
                return service_error(err), False, err
            if not isinstance(req.get("source"), str) or not req["source"]:
                return service_error("missing source"), False, "missing source"

        cache = self.server.cache
        key = cache.key(pipeline, req)
        reply = cache.get(key)
        cached = reply is not None
        if not cached:
            with self.server.jobs:
                reply = pipeline.compile(req) if op == "compile" else pipeline.link(req)
            # A broken host is not a property of the shader: caching it would
            # make one bad minute poison every later run.
            if "ok" in reply:
                cache.put(key, reply)
        reply = dict(reply)
        reply["cached"] = cached
        return reply, cached, self.note(gpu, op, req, reply)

    @staticmethod
    def note(gpu, op, req, reply):
        if op == "link":
            what = "%s link %s+%s" % (gpu, sha12(req.get("vert", "")), sha12(req.get("frag", "")))
        else:
            spec = req.get("blend") or (req.get("blend_key") or "")
            what = "%s %s %s%s" % (gpu, STAGES[req["stage"]], sha12(req["source"]),
                                   (" blend=%s" % spec) if spec else "")
        if reply.get("ok"):
            dw = reply.get("usc_dw")
            return "%s -> ok%s" % (what, "" if dw is None else " usc_dw=%d" % dw)
        kind = "REFUSED" if "ok" in reply else "SERVICE ERROR"
        return "%s -> %s %s" % (what, kind, str(reply.get("error", "")).replace("\n", " ")[:160])


def refuse(sock, message):
    """Answer a connection with one service error and close it without
    blocking for long: this also runs on the accept thread. The request is
    drained a little after the reply, so that closing a socket with unread
    data does not reset the connection before the client reads the reply."""
    deadline = time.monotonic() + 1.0          # the whole refusal, not per call
    try:
        sock.settimeout(1.0)
        send_msg(sock, service_error(message))
        sock.shutdown(socket.SHUT_WR)
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            sock.settimeout(left)
            if not sock.recv(65536):
                break
    except OSError:
        pass


class Server(socketserver.ThreadingTCPServer):
    """One thread per connection, at most `max_connections` of them, and at
    most `jobs` compiler runs at once among them."""

    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, addr, handler, *, pipelines, cache, compiler_timeout,
                 jobs, max_connections, token):
        self.pipelines = pipelines
        self.cache = cache
        self.compiler_timeout = compiler_timeout
        self.jobs = threading.BoundedSemaphore(jobs)
        self.slots = threading.BoundedSemaphore(max_connections)
        self.max_connections = max_connections
        self.token = token.encode("utf-8") if token else b""
        super().__init__(addr, handler)

    def check_token(self, given):
        """None when the request may proceed, else the refusal."""
        if not self.token:
            return None
        if not isinstance(given, str) or not given:
            return REFUSE_TOKEN_MISSING
        if not hmac.compare_digest(given.encode("utf-8"), self.token):
            return REFUSE_TOKEN_WRONG
        return None

    def process_request(self, request, client_address):
        # On the accept thread: a connection past the cap never gets a thread.
        if not self.slots.acquire(blocking=False):
            log("%s:%d %s (limit %d)" % (client_address[0], client_address[1],
                                         REFUSE_CONNECTIONS, self.max_connections))
            refuse(request, REFUSE_CONNECTIONS)
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()


def is_loopback(host):
    """True when every address `host` names is a loopback one."""
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        pass
    try:
        infos = socket.getaddrinfo(host, None)
    except OSError:
        return False
    addrs = {i[4][0].split("%")[0] for i in infos}
    return bool(addrs) and all(ipaddress.ip_address(a).is_loopback for a in addrs)


def exposure_check(host, token, no_auth):
    """(error, warning) for a bind. An error means do not start."""
    if is_loopback(host):
        return None, None
    warning = ("WARNING: listening on %s, not loopback: any host that can reach "
               "this port can run the compilers here" % host)
    if not token and not no_auth:
        return ("refusing to listen on %s without a token: set %s (the client "
                "sends it as \"%s\"), or pass --no-auth to expose the server "
                "without one" % (host, TOKEN_ENV, TOKEN_FIELD)), warning
    return None, warning


def build_pipelines(args):
    """Every pipeline this host can actually run. A GPU whose compiler is
    missing is left out, so a request for it is refused with a message
    instead of failing per shader."""
    out = {}
    try:
        out["pvr"] = PvrPipeline(find_tool(args.tool), args.timeout)
    except Exception as e:                           # noqa: BLE001 - serve what works
        log("pvr pipeline unavailable: %s" % e)
    try:
        # The vivante pipeline borrows the host tool for the ES link rules;
        # a host without it still serves compiles, and its link answers say
        # so as a service error rather than passing everything.
        try:
            viv_tool = find_tool(args.tool)
        except Exception:                            # noqa: BLE001
            viv_tool = ""
        out["vivante"] = VivantePipeline(args.timeout, viv_tool)
    except Exception as e:                           # noqa: BLE001 - serve what works
        log("vivante pipeline unavailable: %s" % e)
    return out


def serve(args):
    token = os.environ.get(TOKEN_ENV, "")
    # Before the pipelines: find_tool may run cargo, and a server that is not
    # going to start must say so at once.
    error, warning = exposure_check(args.host, token, args.no_auth)
    if error:
        sys.stderr.write("shaderc_server: %s\n" % error)
        return 2
    pipelines = build_pipelines(args)
    if not pipelines:
        sys.stderr.write("shaderc_server: no pipeline is available on this host\n")
        return 1
    srv = Server((args.host, args.port), Handler, pipelines=pipelines,
                 cache=Cache(args.cache, args.cache_max_mb << 20),
                 compiler_timeout=args.timeout, jobs=args.jobs,
                 max_connections=args.max_connections, token=token)
    if warning:
        log(warning)
    log("listening on %s:%d" % srv.server_address[:2])
    log("token=%s  jobs=%d  max-connections=%d  cache-max=%s" % (
        "required" if token else "none", args.jobs, args.max_connections,
        ("%d MiB" % args.cache_max_mb) if args.cache_max_mb else "unlimited"))
    log("built from %s" % build_provenance(find_tool(args.tool)))
    for name in sorted(pipelines):
        log("gpu=%s ready" % name)
    log("pco_shc=%s" % (pco_shc_path() or "(the pipeline's default)"))
    log("cache=%s  pco cache=%s" % (args.cache or "(none)", os.environ.get("GLSL2USC_CACHE")))
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        log("stopped")
    return 0


# ---- one-shot client, for probing the server by hand -----------------------

def call(host, port, req, timeout):
    sock = socket.create_connection((host, port), timeout=timeout + IO_GRACE_S)
    try:
        send_msg(sock, req)
        return recv_msg(sock)
    finally:
        sock.close()


def client(args):
    if args.link:
        req = {"gpu": args.gpu, "op": "link",
               "vert": open(args.file).read(), "frag": open(args.link).read()}
    else:
        stage = args.stage or ("vert" if args.file.endswith(".vert") else "frag")
        req = {"gpu": args.gpu, "stage": stage, "source": open(args.file).read()}
        if args.blend:
            req["blend"] = args.blend
    if os.environ.get(TOKEN_ENV):
        req[TOKEN_FIELD] = os.environ[TOKEN_ENV]
    reply = call(args.host, args.port, req, args.timeout)
    if reply is None:
        sys.stderr.write("shaderc_server: no reply\n")
        return 1
    if not reply.get("ok"):
        sys.stderr.write("failed:\n%s\n" % reply.get("error", ""))
        return 1
    reply.pop("blob", None)
    print(json.dumps(reply, indent=2, sort_keys=True))
    return 0


# ---- self-test (tools/check.sh runs it) ------------------------------------

class _FakePipeline(Pipeline):
    """A compiler that takes `delay` seconds and counts how many runs overlap."""

    name = "pvr"
    tool_id = "self-test"

    def __init__(self, delay=0.0):
        self.delay = delay
        self.lock = threading.Lock()
        self.running = 0
        self.peak = 0
        self.calls = 0
        self.seen = []

    def compile(self, req):
        with self.lock:
            self.running += 1
            self.calls += 1
            self.peak = max(self.peak, self.running)
            self.seen.append(dict(req))
        time.sleep(self.delay)
        with self.lock:
            self.running -= 1
        return {"ok": True, "blob": "", "usc_dw": 0, "reflection": {}}

    link = compile


def self_test():
    import tempfile
    results = []

    def check(name, cond, detail=""):
        results.append(bool(cond))
        print("%s  %s%s" % ("ok  " if cond else "FAIL", name, ("  (%s)" % detail) if detail else ""))

    def start(pipeline, token="", jobs=4, max_connections=8, cache=None):
        srv = Server(("127.0.0.1", 0), Handler, pipelines={"pvr": pipeline},
                     cache=cache or Cache(""), compiler_timeout=5, jobs=jobs,
                     max_connections=max_connections, token=token)
        threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": 0.05},
                         daemon=True).start()
        return srv

    def ask(srv, req):
        return call("127.0.0.1", srv.server_address[1], req, 5)

    def shader(n=0, **extra):
        req = {"gpu": "pvr", "stage": "frag", "source": "void main(){} // %d" % n}
        req.update(extra)
        return req

    global log
    real_log, log = log, (lambda msg: None)      # keep the output to the verdicts
    try:
        # The refusals are told apart by their text.
        check("every refusal has its own message", len(set(REFUSALS)) == len(REFUSALS))

        # Token: unset keeps today's behaviour; set, it is checked before
        # anything else and never reaches the pipeline.
        fake = _FakePipeline()
        srv = start(fake)
        check("no token configured: a plain request compiles", ask(srv, shader(1)).get("ok") is True)
        srv.shutdown(); srv.server_close()

        fake = _FakePipeline()
        srv = start(fake, token="s3cret")
        r = ask(srv, shader(2))
        check("token missing: refused as a service error",
              "ok" not in r and r.get("error") == REFUSE_TOKEN_MISSING, r)
        r = ask(srv, shader(3, token="wrong"))
        check("token wrong: refused as a service error",
              "ok" not in r and r.get("error") == REFUSE_TOKEN_WRONG, r)
        check("a refused request never reached the compiler", fake.calls == 0, fake.calls)
        r = ask(srv, shader(4, token="s3cret"))
        check("token right: compiles", r.get("ok") is True, r)
        check("the token is removed before the compiler sees the request",
              fake.seen and TOKEN_FIELD not in fake.seen[-1])
        check("the token is not part of the cache key",
              Cache.key(fake, shader(5)) == Cache.key(fake, shader(5, token="s3cret")))
        srv.shutdown(); srv.server_close()

        # Concurrency: 8 requests at once against a slow compiler, 2 jobs.
        fake = _FakePipeline(delay=0.3)
        srv = start(fake, jobs=2, max_connections=16)
        replies = []
        threads = [threading.Thread(target=lambda n=n: replies.append(ask(srv, shader(10 + n))))
                   for n in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        check("--jobs 2: at most 2 compiler runs overlapped, and 2 did",
              fake.peak == 2 and fake.calls == 8, "peak %d of %d calls" % (fake.peak, fake.calls))
        check("--jobs 2: every waiting request was still answered",
              len(replies) == 8 and all(r.get("ok") for r in replies))
        srv.shutdown(); srv.server_close()

        # Connections: one slot, held by an idle connection.
        #
        # A connection's slot is released by its server thread after the
        # thread has sent the reply and closed its socket, so a client can read
        # its reply (or see EOF) and connect again before that happens. The
        # protocol does not order these events, and a fixed sleep only works
        # when the machine is idle; under gate load the next connection was
        # refused as "too many connections". So the test waits for the
        # server's own count of free slots instead of guessing a delay.
        def free_slots_become(srv, n, timeout=5.0):
            deadline = time.monotonic() + timeout
            while srv.slots._value != n:           # read only: never take a slot here
                if time.monotonic() > deadline:
                    return False
                time.sleep(0.005)
            return True

        fake = _FakePipeline()
        srv = start(fake, max_connections=1)
        port = srv.server_address[1]
        holder = socket.create_connection(("127.0.0.1", port), timeout=5)
        check("an idle connection holds the only slot", free_slots_become(srv, 0))
        r = ask(srv, shader(20))
        check("--max-connections 1: the second connection is refused",
              "ok" not in r and r.get("error") == REFUSE_CONNECTIONS, r)
        holder.close()
        check("the slot comes back when the connection closes",
              free_slots_become(srv, 1) and ask(srv, shader(21)).get("ok") is True)

        # Size: a frame header over the limit gets a reply, not silence. The
        # connection that just compiled shader 21 may still hold the slot.
        free_slots_become(srv, 1)
        sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        sock.sendall(struct.pack(">I", MAX_REQUEST_BYTES + 1))
        r = recv_msg(sock)
        sock.close()
        check("a request over MAX_REQUEST_BYTES is refused",
              r is not None and r.get("error") == REFUSE_TOO_LARGE, r)
        srv.shutdown(); srv.server_close()

        # The cache cap.
        with tempfile.TemporaryDirectory() as d:
            body = {"ok": True, "blob": "x" * 900}
            old = time.time() - 3600
            for n in range(4):                     # left by an earlier server
                path = os.path.join(d, "old%d.json" % n)
                with open(path, "w") as f:
                    json.dump(body, f)
                os.utime(path, (old + n, old + n))
            each = os.path.getsize(os.path.join(d, "old0.json"))
            c = Cache(d, max_bytes=5 * each, protect_s=0)
            c.get("old0")                          # used now: the newest of the old
            c.put("new0", body)
            c.put("new1", body)                    # 6 entries > 5: evict to 4.5
            left = sorted(n[:-5] for n in os.listdir(d))
            check("cache over its cap evicts the least recently used",
                  left == ["new0", "new1", "old0", "old3"], left)

            c = Cache(d, max_bytes=2 * each, protect_s=600)
            for n in range(4):
                c.put("mine%d" % n, body)
            left = sorted(n[:-5] for n in os.listdir(d))
            check("cache never evicts what this process just wrote",
                  left == ["mine0", "mine1", "mine2", "mine3"], left)

        # Exposure: decided before anything is built or bound.
        check("loopback with no token starts",
              exposure_check("127.0.0.1", "", False) == (None, None))
        err, warn = exposure_check("0.0.0.0", "", False)
        check("non-loopback with no token refuses to start", err and TOKEN_ENV in err, err)
        err, warn = exposure_check("0.0.0.0", "", True)
        check("--no-auth starts, with a warning", err is None and warn, warn)
        err, warn = exposure_check("0.0.0.0", "t", False)
        check("a token starts, with a warning", err is None and warn, warn)
        env = {k: v for k, v in os.environ.items() if k != TOKEN_ENV}
        p = subprocess.run([sys.executable, os.path.abspath(__file__), "--host", "0.0.0.0",
                            "--port", "0"], env=env, capture_output=True, text=True, timeout=30)
        check("the server itself exits 2 before building anything",
              p.returncode == 2 and "refusing to listen" in p.stderr and not p.stdout,
              "rc %d" % p.returncode)
    finally:
        log = real_log

    passed = sum(results)
    print("shaderc_server self-test: %d passed, %d failed" % (passed, len(results) - passed))
    return 0 if passed == len(results) else 1


def positive(text):
    n = int(text)
    if n < 1:
        raise argparse.ArgumentTypeError("must be 1 or more")
    return n


def main():
    ap = argparse.ArgumentParser(description="GLSL ES -> GPU shader binary compile server")
    ap.add_argument("--host", default="127.0.0.1",
                    help="bind address (default 127.0.0.1; a board on the LAN needs 0.0.0.0)")
    ap.add_argument("--no-auth", action="store_true",
                    help="allow a non-loopback --host without %s" % TOKEN_ENV)
    ap.add_argument("--jobs", type=positive, default=os.cpu_count() or 1,
                    help="compiler runs at once (default: the CPU count, %d)" % (os.cpu_count() or 1))
    ap.add_argument("--max-connections", type=positive, default=DEFAULT_MAX_CONNECTIONS,
                    help="open connections; one more is refused (default %d)" % DEFAULT_MAX_CONNECTIONS)
    ap.add_argument("--cache-max-mb", type=int, default=DEFAULT_CACHE_MAX_MB,
                    help="cap on --cache's entries, LRU by mtime; 0 = none (default %d)"
                    % DEFAULT_CACHE_MAX_MB)
    ap.add_argument("--self-test", action="store_true", help="run the server's own tests and exit")
    ap.add_argument("--port", type=int, default=DEFAULT_PORT, help="TCP port (default %d)" % DEFAULT_PORT)
    ap.add_argument("--cache", default=os.environ.get("YAKOGL_SHADERC_CACHE_DIR", ""),
                    help="directory for compiled results (default: none)")
    ap.add_argument("--timeout", type=positive, default=COMPILE_TIMEOUT_S,
                    help="seconds one compile may take (default %d)" % COMPILE_TIMEOUT_S)
    ap.add_argument("--pco-shc", default="", help="pco_shc binary (else $PCO_SHC)")
    ap.add_argument("--tool", default="", help="yakogl-tools binary (else $YAKOGL_TOOLS, else cargo)")
    ap.add_argument("--gpu", default="pvr", help="--client: which pipeline to ask (pvr, vivante)")
    ap.add_argument("--client", action="store_true", help="send one shader and print the reflection")
    ap.add_argument("--stage", default="", help="--client: vert or frag")
    ap.add_argument("--blend", default="", help="--client: a glsl2usc --blend spec")
    ap.add_argument("--link", default="", help="--client: link FILE (the vertex shader) with this fragment shader")
    ap.add_argument("file", nargs="?", help="--client: the GLSL file to compile")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if args.cache_max_mb < 0:
        ap.error("--cache-max-mb must be 0 or more")
    if args.pco_shc:
        os.environ["PCO_SHC"] = os.path.abspath(args.pco_shc)
    if args.client:
        if not args.file:
            ap.error("--client needs a GLSL file")
        return client(args)

    if args.cache:
        args.cache = os.path.abspath(args.cache)
        # The compiler's own raw-output cache (GLSL2USC_CACHE) lives beside
        # ours unless the caller placed it somewhere else.
        os.environ.setdefault("GLSL2USC_CACHE", os.path.join(args.cache, "pco"))
    return serve(args)


if __name__ == "__main__":
    sys.exit(main())
