// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! YakoGL host tools (they replaced the Python generators; see tools/README.md).
//!
//!     yakogl-tools glsl2usc [options] inputs...    offline shader table (tests/reference/blobs.rs)
//!     yakogl-tools usc-blob <src> <file> <SYM> [VARIANT]   Mesa's internal USC blobs (tests/reference/pvr_internal.rs)
//!     yakogl-tools femtovg-shaders [<dir> [out]]   FemtoVG 0.26 shader sources
//!     yakogl-tools gl-consts                       src/gl/consts.rs, src/gl/egl_consts.rs
//!     yakogl-tools abi-layout <driver src>         src/abi_layout.rs, src/abi_layout_check.c
//!     yakogl-tools shader-check [--codegen]        the GLSL ES front end, JSON in/JSON out
//!     yakogl-tools online-check                    the in-process front end, same protocol
//!     yakogl-tools online-eval                     run a compiled pair with the IR evaluator
//!     yakogl-tools ir-dump <file.glsl>...          the online front end's IR node shape
//!     yakogl-tools usc-dis [--table <name>|<file>] disassemble a PowerVR USC binary
//!     yakogl-tools etna-dis [<file>|--hex ...]     disassemble a Vivante GC7000 binary
//!     yakogl-tools csbgen-bindings [--check]       src/pvr/csbgen.rs + its wrapper C (bindgen)
//!     yakogl-tools record <w0> <w1> <w2> <w3>      decode a result record (driver/docs/one-method.md section 3)

mod abilayout;
mod csbgen;
mod dump;
mod emit;
mod es100;
mod etnadis;
mod femtovg;
mod glconsts;
mod glsl;
mod glsl2usc;
mod irdump;
// tools/shaders/precompiled.list and the blend-variant spec syntax: the one
// reader, shared with yakogl-precompile (`glsl2usc --sources`). Its
// generator half (dirs, pairs, apps) is unused here.
#[allow(dead_code)]
#[path = "../../precompile/src/list.rs"]
mod list;
mod onlinecheck;
mod onlineeval;
mod py;
// The result record's one definition (driver/docs/one-method.md section 3),
// compiled here as it is in the board layer; recordcmd.rs is the host side.
// The board layer's writer half is unused by a host tool.
mod msaarec;
#[allow(dead_code)]
#[path = "../../../driver/rust/src/board/record.rs"]
mod record;
mod recordcmd;
mod shadercheck;
// The leaf printers of the shader ABI, shared with yakogl-precompile.
mod shc_print;
mod uscblob;
mod uscdis;

/// How a command failed.
#[derive(Debug)]
pub enum Error {
    /// A diagnosed input error (Python's `raise SystemExit(msg)`): printed,
    /// exit status 1; `glsl2usc --skip-failures` skips the shader instead.
    Exit(String),
    /// What was an uncaught exception in Python (I/O errors, malformed
    /// sidecars, values that do not fit the ABI): never skipped.
    Fatal(String),
    /// Bad command line: printed, exit status 2.
    Usage(String),
}

pub type R<T> = Result<T, Error>;

/// The repository root (the tools find their default inputs and outputs
/// relative to it, as the scripts did relative to tools/).
/// `YAKOGL_TOOLS_ROOT` points the tools at another checkout (verification).
pub fn repo_root() -> String {
    match py::env_set("YAKOGL_TOOLS_ROOT") {
        Some(r) => py::abspath(&r),
        None => py::normpath(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")),
    }
}

const USAGE: &str = "usage: yakogl-tools <command> [args...]

commands:
  glsl2usc          GLSL ES -> PowerVR USC table (tests/reference/blobs.rs); --help for options
  usc-blob          one internal shader's Mesa USC block in tests/reference/pvr_internal.rs
  femtovg-shaders   write the FemtoVG 0.26 shader sources into tools/shaders/femtovg
  gl-consts         regenerate src/gl/consts.rs and src/gl/egl_consts.rs
  abi-layout        regenerate src/abi_layout.rs and src/abi_layout_check.c
  shader-check      the GLSL ES front end as a batch service (JSON on stdin/stdout)
  online-check      the same, for the in-process front end (src/shaderc/online/)
  online-eval       compile a vertex/fragment pair online and RUN it with the IR evaluator
  ir-dump           print the online front end's IR node shape for a GLSL file
  usc-dis           disassemble a PowerVR USC binary (a table row, or a file)
  etna-dis          disassemble a Vivante GC7000 binary (a file, or --hex words)
  csbgen-bindings   regenerate src/pvr/csbgen.rs and its wrapper C (--check: compare)
  record            decode a result record (driver/rust/src/board/record.rs; --help for the forms)";
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let rest = &args[1..];
    let r = match cmd.as_str() {
        "glsl2usc" => glsl2usc::main(rest),
        "usc-blob" => uscblob::main(rest),
        "femtovg-shaders" => femtovg::main(rest),
        "gl-consts" => glconsts::main(rest),
        "abi-layout" => abilayout::main(rest),
        "shader-check" => shadercheck::main(rest),
        "online-check" => onlinecheck::main(rest),
        "online-eval" => onlineeval::main(rest),
        "ir-dump" => irdump::main(rest),
        "usc-dis" => uscdis::main(rest),
        "etna-dis" => etnadis::main(rest),
        "csbgen-bindings" => csbgen::main(rest),
        "record" => recordcmd::main(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(Error::Usage(format!("{USAGE}\n\nunknown command '{other}'"))),
    };
    match r {
        Ok(()) => {}
        Err(Error::Exit(m)) => {
            eprintln!("{m}");
            std::process::exit(1);
        }
        Err(Error::Fatal(m)) => {
            eprintln!("yakogl-tools {cmd}: error: {m}");
            std::process::exit(1);
        }
        Err(Error::Usage(m)) => {
            eprintln!("{m}");
            std::process::exit(2);
        }
    }
}
