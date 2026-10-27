// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Integer samplers on the GC7000: an `isampler*` / `usampler*` look-up is
//! the float look-up of the same shape.
//!
//! The texture engine returns an integer format's texels as the integers
//! themselves in the destination register (sign-extended for a signed format
//! by the descriptor's CONFIG2, `vivante/texture.rs` `gc_texfmt`), and the
//! instruction does not name the result type: Mesa 26.2.1's etnaviv emits the
//! same `texld` / `texldl` / `texldb` / `txf` for every sampler type
//! (etnaviv_compiler_nir_emit.c:206-247, from etnaviv_compiler_nir.c:538-570;
//! etnaviv_nir_lower_texture.c:449-499 has no integer case). The selector
//! (`online/select.rs`) matches the float result type only, so this pass,
//! run on the GC7000's module before selection, gives every non-shadow
//! integer look-up that type. Nothing else changes: the value's readers are
//! the front end's own conversions (`vec4(texture(u, c))` is a `u2f`) and
//! integer operations, which read the register as the integers it holds. A
//! shape the float path does not select either (an offset, a gradient) is
//! still refused by name there. docs/es3/gc-formats.md section 3.

use super::super::arena::{AVec, Arena};
use super::super::ir::{self, Op, ScalarType};
use super::super::lower::Lowered;
use super::super::R;

/// `l` with every integer-result look-up given the float result type. A
/// module without one comes back as it was.
pub fn float_typed_lookups<'a>(a: &Arena<'a>, l: Lowered<'a>) -> R<'a, Lowered<'a>> {
    let int_lookup = |op: &Op| matches!(op, Op::Tex { shadow: false, ret: ScalarType::I32 | ScalarType::U32, .. });
    if !l.module.instrs.iter().any(int_lookup) {
        return Ok(l);
    }
    let mut instrs: AVec<Op> = AVec::with_capacity(a, l.module.instrs.len())?;
    for op in l.module.instrs.iter() {
        let mut op = *op;
        if let Op::Tex { ref mut ret, shadow: false, .. } = op {
            if matches!(*ret, ScalarType::I32 | ScalarType::U32) {
                *ret = ScalarType::F32;
            }
        }
        instrs.push(a, op)?;
    }
    /* ids, widths and nodes are the module's: one instruction for one */
    Ok(Lowered { module: ir::Module { instrs: instrs.into_slice(), ..l.module }, reflection: l.reflection })
}
