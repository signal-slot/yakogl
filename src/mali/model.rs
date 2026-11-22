// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Which Valhall GPU the backend draws for, and the tile buffer that model
//! has: the arch-10 rows of Mesa 26.2.1's model table
//! (src/panfrost/model/pan_model.c:88-100), looked up as `pan_get_model`
//! does (pan_model.c:139-149) from
//! `GPU_ID` and the variant, the low byte of `CORE_FEATURES`
//! (src/panfrost/lib/kmod/panthor_kmod.c:162, `core_features & 0xff`).
//!
//! The only thing the frame builder takes from the model is the tile-buffer
//! budget ([`TileBudget`]): the G310v1-v3 have half the G610's tile buffer,
//! which moves the tile size, the colour buffer allocation, the render
//! targets' tile-buffer offsets and the tiler hierarchy mask of frames of
//! 12 bytes a pixel or more (docs/notes/mali-g310-vs-g610.md). The shader
//! side does not depend on the model (the same note, section 3).

/// `PAN_PROD_ID(arch major, arch minor, product major)` (pan_model.h:39-40).
pub const fn prod_id(arch_major: u32, arch_minor: u32, product_major: u32) -> u32 {
    arch_major << 16 | arch_minor << 8 | product_major
}

/// `pan_prod_id` of a `GPU_ID` of arch 6 or later (pan_model.h:24-27,
/// 120-127): arch major bits 31:28, arch minor 27:24, product major 19:16.
pub const fn prod_id_of(gpu_id: u32) -> u32 {
    prod_id(gpu_id >> 28, (gpu_id >> 24) & 0xf, (gpu_id >> 16) & 0xf)
}

/// The model's tile buffer as the table gives it (`MODEL_TB_SIZES`, bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileBuffer {
    pub colour: u32,
    pub z: u32,
}

/// The budgets a frame's tile is sized for (`tile_buf_budget` and
/// `z_tile_buf_budget`, gallium pan_job.c:472-473): half of each tile
/// buffer on arch 10, where pipelining can be turned off for the other half
/// (`pan_query_optimal_tib_size` and `pan_query_optimal_z_tib_size`,
/// src/panfrost/lib/pan_props.h:47-72, `arch > 4 && arch != 6`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileBudget {
    pub colour: u32,
    pub z: u32,
}

impl TileBudget {
    /// The NanoPC-T6's Mali-G610: 32768 / 2 and 16384 / 2.
    pub const G610: TileBudget = G610.tile_buffer.budget();
}

impl TileBuffer {
    /// The arch-10 budgets of this tile buffer.
    pub const fn budget(self) -> TileBudget {
        TileBudget { colour: self.colour / 2, z: self.z / 2 }
    }
}

/// One row of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    /// Mesa's name (`GL_RENDERER`'s first word, gallium pan_screen.c:1051-1054).
    pub name: &'static str,
    /// Our `GL_RENDERER` on this model, NUL-terminated: `Arm Mali-<name>
    /// (YakoGL)`, Mesa's model name in our form (Mesa says `Mali-G310v2 MC4
    /// (Panfrost)`, docs/notes/mali-g310-vs-g610.md). dEQP records it in
    /// every result file, so results of two models can be told apart.
    pub renderer: &'static [u8],
    pub prod_id: u32,
    /// `CORE_FEATURES & 0xff`.
    pub variant: u32,
    pub tile_buffer: TileBuffer,
}

impl Model {
    pub const fn tile_budget(&self) -> TileBudget {
        self.tile_buffer.budget()
    }
}

/// A row, its renderer string made from its name at compile time.
macro_rules! row {
    ($name:literal, $prod_id:expr, $variant:expr, $colour:expr, $z:expr) => {
        Model {
            name: $name,
            renderer: concat!("Arm Mali-", $name, " (YakoGL)\0").as_bytes(),
            prod_id: $prod_id,
            variant: $variant,
            tile_buffer: TileBuffer { colour: $colour, z: $z },
        }
    };
}

/// pan_model.c:88: `VALHALL_MODEL(PAN_PROD_ID(10, 8, 7), 0, "G610", ...,
/// MODEL_TB_SIZES(32768, 16384))`.
pub const G610: Model = row!("G610", prod_id(10, 8, 7), 0, 32768, 16384);

/// The models the backend draws for: pan_model.c:88 (the G610) and :91-99
/// (the G310, `PAN_PROD_ID(10, 12, 4)`, variants 0-4: "G310v1".."G310v5";
/// v1-v3 `MODEL_TB_SIZES(16384, 8192)`, v4-v5 `(32768, 16384)`). The other
/// arch-10 products (G710, G510) have no row in Mesa 26.2.1's table: Mesa
/// does not draw for them, and neither does this backend.
pub const MODELS: [Model; 6] = [
    G610,
    row!("G310v1", prod_id(10, 12, 4), 0, 16384, 8192),
    row!("G310v2", prod_id(10, 12, 4), 1, 16384, 8192),
    row!("G310v3", prod_id(10, 12, 4), 2, 16384, 8192),
    row!("G310v4", prod_id(10, 12, 4), 3, 32768, 16384),
    row!("G310v5", prod_id(10, 12, 4), 4, 32768, 16384),
];

/// The model of the GPU whose `GPU_ID` and `CORE_FEATURES` are these, as
/// `pan_get_model(gpu_id, core_features & 0xff)` finds it: the product id
/// and the variant both equal; `None` for a GPU Mesa 26.2.1 has no row
/// for. The version fields of `GPU_ID` (arch revision, rXpY, status) do not
/// decide.
pub fn model(gpu_id: u32, core_features: u32) -> Option<&'static Model> {
    let (prod, variant) = (prod_id_of(gpu_id), core_features & 0xff);
    MODELS.iter().find(|m| m.prod_id == prod && m.variant == variant)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows against the table's lines (pan_model.c:88-100) and the
    /// lookup against the GPU_IDs the kernel side reads (the T6's
    /// 0xa8670005; the i.MX 95's 0xac740001, docs/notes/mali-gpu-model.md)
    /// and drm_shim_ref.sh's `--gpu` (`PAN_GPU_ID=a867`, `ac74:<variant>`).
    #[test]
    fn the_models_are_mesa_s_rows_and_the_lookup_is_pan_get_model() {
        assert_eq!(prod_id_of(0xa867_0005), prod_id(10, 8, 7));
        assert_eq!(prod_id_of(0xac74_0001), prod_id(10, 12, 4));
        assert_eq!(model(0xa867_0005, 0), Some(&G610));
        assert_eq!(model(0xa867_0000, 0x1200), Some(&G610), "only the low byte is the variant");
        assert_eq!(model(0xa867_0005, 1), None, "Mesa has no G610 variant 1");
        let budgets: Vec<_> = (0..6).map(|v| model(0xac74_0001, v).map(|m| (m.name, m.tile_budget()))).collect();
        let (small, large) = (TileBudget { colour: 8192, z: 4096 }, TileBudget { colour: 16384, z: 8192 });
        assert_eq!(
            budgets,
            [
                Some(("G310v1", small)),
                Some(("G310v2", small)),
                Some(("G310v3", small)),
                Some(("G310v4", large)),
                Some(("G310v5", large)),
                None
            ]
        );
        assert_eq!(TileBudget::G610, large);
        /* GL_RENDERER: the NanoPC-T6's bytes stay as its dEQP records have
         * them; each G310 variant names itself */
        let renderers: Vec<&[u8]> = MODELS.iter().map(|m| m.renderer).collect();
        assert_eq!(
            renderers,
            [
                &b"Arm Mali-G610 (YakoGL)\0"[..],
                b"Arm Mali-G310v1 (YakoGL)\0",
                b"Arm Mali-G310v2 (YakoGL)\0",
                b"Arm Mali-G310v3 (YakoGL)\0",
                b"Arm Mali-G310v4 (YakoGL)\0",
                b"Arm Mali-G310v5 (YakoGL)\0"
            ]
        );
        /* the G710 (10.?.2) and G510 (10.?.3) have no row */
        assert_eq!(model(0xa002_0000, 0), None);
        assert_eq!(model(0xa003_0000, 0), None);
    }
}
