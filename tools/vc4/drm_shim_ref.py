#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mesa's own GL driver for the VideoCore IV (gallium vc4), run without a GPU
on Mesa's drm-shim, as the host reference for the Raspberry Pi Zero 2 W port.
The method of tools/v3d/drm_shim_ref.sh; tools/vc4/README.md says what the
outputs are and what was checked.

    tools/vc4/drm_shim_ref.py build <mesa-X.Y.Z.tar.xz> <work dir>
        check the tarball against third_party/mesa-X.Y.Z/UPSTREAM (sha256),
        extract it into <work dir>/mesa-X.Y.Z (deleted first), make the three
        edits below, build the vc4 driver, EGL, GLES 2 and the drm-shim, and
        install into <work dir>/inst.
    tools/vc4/drm_shim_ref.py run <work dir> <case> <out prefix>
        <case> is clear or triangle (tools/v3d/gl_ref.py, unchanged). Draws it
        twice on the shim with MESA_LOADER_DRIVER_OVERRIDE=vc4, once with
        VC4_DEBUG=cl and once with VC4_DEBUG=qpu,qir, and writes
          <out prefix>.rec.bin     what the driver handed the kernel, raw
                                   (tools/vc4/vc4_submit.py reads it)
          <out prefix>.cl.txt      Mesa's text dump of the binner list
          <out prefix>.qpu.txt     Mesa's dump of each shader (QIR, QPU words)
                                   and of the uniform words the draw wrote;
                                   not written when the case has no shader
          <out prefix>.decode.txt  vc4_submit.py's reading of .rec.bin
          <out prefix>.stdout      gl_ref.py's two lines
        The two draws must record the same bytes, or the run fails.
    tools/vc4/drm_shim_ref.py shaders <work dir> <vertex.glsl> <fragment.glsl> <out prefix>
        the same as run, with the triangle drawn by this shader pair
        (tools/vc4/vc4_shc.py is the front end of this form).

What build changes in the extracted tree (edit_mesa below; each edit stops
the build when its place in the source is not found exactly once), as
tools/v3d/drm_shim_ref.sh adds its 7.1.10 GPU to the V3D shim:

1. The recorder. Mesa 26.2.1's vc4 shim has no handler for DRM_VC4_SUBMIT_CL
   at all (src/broadcom/drm-shim/vc4_noop.c: the ioctl table lacks it, so the
   shim prints "DRM_SHIM: unhandled driver DRM ioctl 0", the submit fails and
   the driver warns "Draw call returned ... Expect corruption."). build adds
   one that accepts the submit and, when VC4_SHIM_RECORD names a file,
   appends to it

       CRBO  struct drm_vc4_create_bo, after the handle is assigned
       SHBO  struct drm_vc4_create_shader_bo (data pointer zeroed), then the
             QPU words the driver passed
       SUBM  struct drm_vc4_submit_cl (the four user pointers zeroed), then
             the binner list, the shader records, the uniforms and the BO
             handle table, each as long as the struct says

   each as a 4-byte tag, a little-endian u32 length and that many bytes.
2. DRM_VC4_WAIT_SEQNO and DRM_VC4_WAIT_BO answer 0 (the shim's own no-op
   handler): the driver waits on a BO before it maps one a job used.
3. Mesa's own dump of each uniform word as a draw writes it
   (src/gallium/drivers/vc4/vc4_uniforms.c, there under `if (false)`) is
   switched on with VC4_DEBUG=qpu, naming the shader by its number instead of
   its address.

Nothing of Mesa is kept in this repository.

Needs meson, ninja, a C and a C++ compiler, python3 (3.12 or later) with mako
and pyyaml, bison, flex, libdrm, expat and zlib (the packages Mesa's build
lists).
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
GL_REF = ROOT / "tools" / "v3d" / "gl_ref.py"
SHIM = "libvc4_noop_drm_shim.so"

# Added to src/broadcom/drm-shim/vc4_noop.c, before its ioctl table.
RECORDER = r"""
/* YakoGL (tools/vc4/drm_shim_ref.py): record what the driver hands the kernel. */
#include <stdint.h>
#include <string.h>

static void
yakogl_record(const char *tag, const void *head, uint32_t head_size,
              const void *const *parts, const uint32_t *sizes, int count)
{
        const char *path = getenv("VC4_SHIM_RECORD");
        uint32_t len = head_size;
        FILE *f;

        if (!path)
                return;
        f = fopen(path, "ab");
        if (!f) {
                fprintf(stderr, "VC4_SHIM_RECORD: cannot open %s\n", path);
                abort();
        }
        for (int i = 0; i < count; i++)
                len += sizes[i];
        fwrite(tag, 1, 4, f);
        fwrite(&len, 4, 1, f);
        fwrite(head, 1, head_size, f);
        for (int i = 0; i < count; i++)
                fwrite(parts[i], 1, sizes[i], f);
        if (fclose(f) != 0) {
                fprintf(stderr, "VC4_SHIM_RECORD: write to %s failed\n", path);
                abort();
        }
}

static int
yakogl_create_bo(int fd, unsigned long request, void *arg)
{
        int ret = vc4_ioctl_create_bo(fd, request, arg);

        if (ret == 0)
                yakogl_record("CRBO", arg, sizeof(struct drm_vc4_create_bo),
                              NULL, NULL, 0);
        return ret;
}

static int
yakogl_create_shader_bo(int fd, unsigned long request, void *arg)
{
        struct drm_vc4_create_shader_bo *create = arg;
        int ret = vc4_ioctl_create_shader_bo(fd, request, arg);

        if (ret == 0) {
                struct drm_vc4_create_shader_bo head = *create;
                const void *parts[1] = { (const void *)(uintptr_t)create->data };
                uint32_t sizes[1] = { create->size };

                head.data = 0;
                yakogl_record("SHBO", &head, sizeof(head), parts, sizes, 1);
        }
        return ret;
}

static int
yakogl_submit_cl(int fd, unsigned long request, void *arg)
{
        struct drm_vc4_submit_cl *submit = arg;
        struct drm_vc4_submit_cl head = *submit;
        const void *parts[4] = {
                (const void *)(uintptr_t)submit->bin_cl,
                (const void *)(uintptr_t)submit->shader_rec,
                (const void *)(uintptr_t)submit->uniforms,
                (const void *)(uintptr_t)submit->bo_handles,
        };
        uint32_t sizes[4] = {
                submit->bin_cl_size,
                submit->shader_rec_size,
                submit->uniforms_size,
                submit->bo_handle_count * 4,
        };

        head.bin_cl = 0;
        head.shader_rec = 0;
        head.uniforms = 0;
        head.bo_handles = 0;
        yakogl_record("SUBM", &head, sizeof(head), parts, sizes, 4);
        return 0;
}

"""
TABLE = "static ioctl_fn_t driver_ioctls[] = {\n"
# the table's entries the recorder wraps, and the one it adds
TABLE_EDITS = (
    ("[DRM_VC4_CREATE_BO] = vc4_ioctl_create_bo,", "[DRM_VC4_CREATE_BO] = yakogl_create_bo,"),
    ("[DRM_VC4_CREATE_SHADER_BO] = vc4_ioctl_create_shader_bo,",
     "[DRM_VC4_CREATE_SHADER_BO] = yakogl_create_shader_bo,"),
)
# Nothing runs on the shim, so there is never anything to wait for (the driver
# waits on a BO before it maps one that a job used: a texture upload).
TABLE_ADD = ("        [DRM_VC4_SUBMIT_CL] = yakogl_submit_cl,\n"
             "        [DRM_VC4_WAIT_SEQNO] = vc4_ioctl_noop,\n"
             "        [DRM_VC4_WAIT_BO] = vc4_ioctl_noop,\n")
# vc4_uniforms.c, vc4_write_uniforms: Mesa's own dump of each uniform word as it
# is written, which is there under `if (false)`. Switched on with VC4_DEBUG=qpu,
# and the shader named by its number instead of its address (which differs
# from run to run).
UNIFORM_DUMP_EDITS = (
    ("                if (false) {\n", "                if (VC4_DBG(QPU)) {\n"),
    ('mesa_logd("%p/%d: 0x%08x %s",\n                                  shader, i, written_val, desc);',
     'mesa_logi("uniform %d/%d: 0x%08x %s",\n                                  (int)shader->program_id, i, written_val, desc);'),
)


def die(msg):
    sys.exit("drm_shim_ref: " + msg)


def edit(path, edits):
    """Replace each (old, new) in a Mesa source; every old must be there exactly once."""
    text = path.read_text()
    for old, new in edits:
        if text.count(old) != 1:
            die("%s: expected one %r, found %d: this Mesa is not the one these edits were written for"
                % (path, old.strip(), text.count(old)))
        text = text.replace(old, new)
    path.write_text(text)


def edit_mesa(src):
    shim_c = src / "src" / "broadcom" / "drm-shim" / "vc4_noop.c"
    if "DRM_VC4_SUBMIT_CL" in shim_c.read_text():
        die("%s already names DRM_VC4_SUBMIT_CL: this Mesa is not the one the recorder was written for" % shim_c)
    edit(shim_c, TABLE_EDITS + ((TABLE, RECORDER + TABLE + TABLE_ADD),))
    edit(src / "src" / "gallium" / "drivers" / "vc4" / "vc4_uniforms.c", UNIFORM_DUMP_EDITS)


def upstream_sha256(name):
    """The tarball-sha256 of third_party/<name>/UPSTREAM, or None without that file."""
    upstream = ROOT / "third_party" / name / "UPSTREAM"
    if not upstream.is_file():
        return None
    m = re.search(r"^tarball-sha256:\s*([0-9a-f]{64})\s*$", upstream.read_text(), re.M)
    return m.group(1) if m else None


def build(tarball, work):
    tarball = Path(tarball).resolve()
    if not tarball.name.endswith(".tar.xz"):
        die("%s is not a .tar.xz" % tarball)
    name = tarball.name[:-len(".tar.xz")]
    want = upstream_sha256(name)
    if want is None:
        die("third_party/%s/UPSTREAM names no tarball-sha256: %s is not the vendored Mesa version" % (name, name))
    h = hashlib.sha256()
    with open(tarball, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    if h.hexdigest() != want:
        die("%s has sha256 %s, third_party/%s/UPSTREAM says %s" % (tarball, h.hexdigest(), name, want))
    work = Path(work)
    work.mkdir(parents=True, exist_ok=True)
    work = work.resolve()
    src, inst = work / name, work / "inst"
    for d in (src, inst):
        if d.exists():
            shutil.rmtree(d)
    with tarfile.open(tarball) as tar:
        tar.extractall(work, filter="data")
    edit_mesa(src)
    subprocess.run(
        ["meson", "setup", str(src / "build"), str(src), "-Dbuildtype=debug", "-Dgallium-drivers=vc4",
         "-Dvulkan-drivers=", "-Dtools=drm-shim", "-Dplatforms=", "-Dglx=disabled", "-Degl=enabled",
         "-Dgbm=disabled", "-Dgles1=disabled", "-Dgles2=enabled", "-Dopengl=false", "-Dglvnd=disabled",
         "-Dllvm=disabled", "-Dprefix=" + str(inst)], check=True)
    subprocess.run(["ninja", "-C", str(src / "build"), "install"], check=True)
    libdir = find_libdir(work)
    print("drm_shim_ref: %s installed in %s (%s in %s)" % (name, inst, SHIM, libdir.relative_to(work)))


def find_libdir(work):
    """meson's libdir: lib64 (Fedora), lib/<multiarch> (Debian, Ubuntu) or lib."""
    inst = Path(work).resolve() / "inst"
    for d in [inst / "lib64", *sorted((inst / "lib").glob("*-linux-gnu")), inst / "lib"]:
        if (d / SHIM).is_file():
            return d
    die("%s has no build (run the build form)" % work)


def mesa_tree(work):
    """The extracted Mesa tree of a work directory."""
    trees = sorted(p for p in Path(work).resolve().glob("mesa-*") if p.is_dir())
    if len(trees) != 1:
        die("%s holds %d mesa-* trees, not 1" % (work, len(trees)))
    return trees[0]


def draw(libdir, debug, record, draw_args):
    """One draw on the shim. Returns (stdout, stderr)."""
    env = dict(os.environ)
    env.update(LD_PRELOAD=str(libdir / SHIM), LD_LIBRARY_PATH=str(libdir),
               MESA_LOADER_DRIVER_OVERRIDE="vc4", MESA_SHADER_CACHE_DISABLE="true",
               VC4_DEBUG=debug, VC4_SHIM_RECORD=str(record))
    # Mesa's log lines must not depend on the caller's settings
    for name in ("MESA_DEBUG", "MESA_LOG_FILE", "MESA_LOG", "MESA_LOG_PREFIX", "LIBGL_DEBUG", "EGL_LOG_LEVEL"):
        env.pop(name, None)
    if record.exists():
        record.unlink()
    p = subprocess.run([sys.executable, "-u", *draw_args], env=env, capture_output=True, text=True)
    if p.returncode != 0:
        if record.exists():
            record.unlink()
        sys.stderr.write(p.stderr[-2000:])
        die("the draw failed (VC4_DEBUG=%s)" % debug)
    if not record.is_file():
        die("the shim recorded nothing (VC4_DEBUG=%s): is %s built by this script?" % (debug, SHIM))
    return p.stdout, p.stderr


def run(work, out, draw_args):
    import vc4_submit

    libdir = find_libdir(work)
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    rec, rec2 = Path(str(out) + ".rec.bin"), Path(str(out) + ".rec2.bin")
    stdout, cl = draw(libdir, "cl", rec, draw_args)
    stdout2, qpu = draw(libdir, "qpu,qir", rec2, draw_args)
    same = rec.read_bytes() == rec2.read_bytes() and stdout == stdout2
    rec2.unlink()
    if not same:
        die("the two draws did not record the same bytes")
    for text in (cl, qpu):
        if "DRM_SHIM: unhandled" in text or "Expect corruption" in text:
            sys.stderr.write(text[-2000:])
            die("the shim refused an ioctl of the driver")
    Path(str(out) + ".stdout").write_text(stdout)
    Path(str(out) + ".cl.txt").write_text(cl)
    qpu_txt = Path(str(out) + ".qpu.txt")
    if qpu:
        qpu_txt.write_text(qpu)
    elif qpu_txt.exists():  # a case that compiles no shader (clear) has no dump
        qpu_txt.unlink()
    record = vc4_submit.Record(rec.read_bytes())
    # the opcode table vc4_submit.py walks the list with, against Mesa's header
    packet_h = mesa_tree(work) / "src" / "gallium" / "drivers" / "vc4" / "kernel" / "vc4_packet.h"
    vc4_submit.check_packet_table(packet_h.read_text())
    Path(str(out) + ".decode.txt").write_text(record.decode())
    sys.stdout.write("".join(ln + "\n" for ln in stdout.splitlines() if ln.startswith("gl_ref:")))
    print("drm_shim_ref: %s.rec.bin %s" % (out, record.summary()))


def draw_shaders(libdir, vs, fs):
    """The child of the shaders form: gl_ref.py's triangle with another pair."""
    sys.path.insert(0, str(GL_REF.parent))
    import gl_ref

    gl_ref.VS = Path(vs).read_bytes()
    gl_ref.FS = Path(fs).read_bytes()
    sys.argv = [str(GL_REF), libdir, "triangle"]
    return gl_ref.main()


def main():
    a = sys.argv[1:]
    sys.path.insert(0, str(HERE))
    if len(a) == 3 and a[0] == "build":
        build(a[1], a[2])
    elif len(a) == 4 and a[0] == "run" and a[2] in ("clear", "triangle"):
        run(a[1], a[3], [str(GL_REF), str(find_libdir(a[1])), a[2]])
    elif len(a) == 5 and a[0] == "shaders":
        run(a[1], a[4], [str(Path(__file__).resolve()), "draw-shaders", str(find_libdir(a[1])),
                         str(Path(a[2]).resolve()), str(Path(a[3]).resolve())])
    elif len(a) == 4 and a[0] == "draw-shaders":
        return draw_shaders(a[1], a[2], a[3])
    else:
        sys.stderr.write(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ValueError as e:  # vc4_submit.py refused the record
        die(str(e))
