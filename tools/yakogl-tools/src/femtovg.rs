// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools femtovg-shaders [<femtovg-0.26 src/renderer/opengl dir> [outdir]]`
//!
//! Reproduce, byte for byte, the GLSL sources Slint's FemtoVG renderer
//! assembles at runtime (femtovg 0.26), so the offline shader table (keyed by
//! the SHA-256 of the exact glShaderSource text) can serve them. FemtoVG
//! builds each program in MainProgram::new as
//!   VS = format!("{}\n{}\n{}", "#version 100", "#define EDGE_AA 1", main-vs.glsl)
//!   FS = format!("{}\n{}\n{}\n{}", "#version 100", "#define EDGE_AA 1",
//!                "#define SELECT_SHADER N[\n#define ENABLE_GLYPH_TEXTURE]", main-fs.glsl)
//! (antialias is hardcoded true, so EDGE_AA is always defined). The no_std
//! fork also picks a NO_DISCARD program for every draw whose stroke
//! threshold is negative, so both permutations are written, and a
//! NO_SCISSOR one of those for every fill drawn without a scissor (no glyph
//! texture; written as `<name>.nodiscard.noscissor.frag`), whose body is
//! main-fs.glsl with the scissor-mask line replaced by `scissor = 1.0`, the
//! same replacement the fork's program.rs makes.

use crate::py;
use crate::R;

pub fn main(argv: &[String]) -> R<()> {
    let fvg = match argv.first() {
        Some(p) => p.clone(),
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
            format!("{home}/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/femtovg-0.26.0/src/renderer/opengl")
        }
    };
    let out = match argv.get(1) {
        Some(p) => p.clone(),
        None => py::join(&crate::repo_root(), "tools/shaders/femtovg"),
    };
    std::fs::create_dir_all(&out).map_err(|e| crate::Error::Fatal(format!("mkdir {out}: {e}")))?;
    let (v, defs) = ("#version 100", "#define EDGE_AA 1");
    // open().read(): text mode, so CRLF sources read as LF
    let vs = py::read_text(&py::join(&fvg, "main-vs.glsl"))?;
    let fs = py::read_text(&py::join(&fvg, "main-fs.glsl"))?;
    py::write_text(&py::join(&out, "main.vert"), &format!("{v}\n{defs}\n{vs}"))?;
    let progs = [
        (0, "fill_gradient"),
        (1, "fill_image"),
        (2, "stencil"),
        (3, "fill_image_gradient"),
        (4, "filter_image"),
        (5, "fill_color"),
        (6, "texture_copy_unclipped"),
        (8, "fill_gradient_conic"),
        (9, "fill_image_gradient_conic"),
    ];
    let glyph_ok = [0, 1, 3, 5, 8, 9];
    let mut count = 0;
    for (n, name) in progs {
        for nd in [false, true] {
            if nd && n == 6 {
                continue; // texture copy has no stroke test to drop
            }
            let suffix = if nd { ".nodiscard" } else { "" };
            let nd_def = if nd { "\n#define NO_DISCARD" } else { "" };
            py::write_text(
                &py::join(&out, &format!("{name}{suffix}.frag")),
                &format!("{v}\n{defs}\n#define SELECT_SHADER {n}\n{nd_def}\n{fs}"),
            )?;
            count += 1;
            if nd && glyph_ok.contains(&n) {
                py::write_text(
                    &py::join(&out, &format!("{name}{suffix}.noscissor.frag")),
                    &format!(
                        "{v}\n{defs}\n#define SELECT_SHADER {n}\n{nd_def}\n#define NO_SCISSOR\n{}",
                        fs.replace("float scissor = scissorMask(fpos);", "float scissor = 1.0;")
                    ),
                )?;
                count += 1;
            }
            if glyph_ok.contains(&n) {
                py::write_text(
                    &py::join(&out, &format!("{name}.glyph{suffix}.frag")),
                    &format!("{v}\n{defs}\n#define SELECT_SHADER {n}\n#define ENABLE_GLYPH_TEXTURE{nd_def}\n{fs}"),
                )?;
                count += 1;
            }
        }
    }
    println!("wrote {count} FemtoVG shader sources to {out}");
    Ok(())
}
