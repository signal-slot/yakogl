// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000 registers, command words and feature bits the kernel side uses.
//!
//! Every name and value is the etna_viv rnndb's (MIT), as generated into
//! `driver/src/vivante/hw/{state_hi,state,cmdstream,common}.xml.h` by
//! rules-ng-ng headergen: same names, same numbers, only the ones this module
//! needs. The test at the bottom reads those headers and checks every
//! constant here against its `#define`, so a transcription error cannot pass.
//! Comments say what galcore (NXP's Vivante kernel driver, MIT) calls the
//! same register where that helps a reader of its code, which writes offsets
//! as bare hex.

// ---- host interface (state_hi.xml.h, domain VIVS) ----

pub const VIVS_HI_CLOCK_CONTROL: u32 = 0x0000_0000;
pub const VIVS_HI_CLOCK_CONTROL_FSCALE_VAL__MASK: u32 = 0x0000_01fc;
pub const VIVS_HI_CLOCK_CONTROL_FSCALE_VAL__SHIFT: u32 = 2;
pub const VIVS_HI_CLOCK_CONTROL_FSCALE_CMD_LOAD: u32 = 0x0000_0200;
pub const VIVS_HI_CLOCK_CONTROL_DISABLE_DEBUG_REGISTERS: u32 = 0x0000_0800;
pub const VIVS_HI_CLOCK_CONTROL_SOFT_RESET: u32 = 0x0000_1000;
pub const VIVS_HI_CLOCK_CONTROL_IDLE_3D: u32 = 0x0001_0000;
pub const VIVS_HI_CLOCK_CONTROL_IDLE_2D: u32 = 0x0002_0000;
pub const VIVS_HI_CLOCK_CONTROL_ISOLATE_GPU: u32 = 0x0008_0000;

pub const fn vivs_hi_clock_control_fscale_val(x: u32) -> u32 {
    (x << VIVS_HI_CLOCK_CONTROL_FSCALE_VAL__SHIFT) & VIVS_HI_CLOCK_CONTROL_FSCALE_VAL__MASK
}

pub const VIVS_HI_IDLE_STATE: u32 = 0x0000_0004;
pub const VIVS_HI_IDLE_STATE_FE: u32 = 0x0000_0001;
pub const VIVS_HI_IDLE_STATE_DE: u32 = 0x0000_0002;
pub const VIVS_HI_IDLE_STATE_PE: u32 = 0x0000_0004;
pub const VIVS_HI_IDLE_STATE_SH: u32 = 0x0000_0008;
pub const VIVS_HI_IDLE_STATE_PA: u32 = 0x0000_0010;
pub const VIVS_HI_IDLE_STATE_SE: u32 = 0x0000_0020;
pub const VIVS_HI_IDLE_STATE_RA: u32 = 0x0000_0040;
pub const VIVS_HI_IDLE_STATE_TX: u32 = 0x0000_0080;
pub const VIVS_HI_IDLE_STATE_MC: u32 = 0x0000_4000;
pub const VIVS_HI_IDLE_STATE_AXI_LP: u32 = 0x8000_0000;

pub const VIVS_HI_AXI_CONFIG: u32 = 0x0000_0008;
pub const VIVS_HI_AXI_CONFIG_AWCACHE__SHIFT: u32 = 8;
pub const VIVS_HI_AXI_CONFIG_AWCACHE__MASK: u32 = 0x0000_0f00;
pub const VIVS_HI_AXI_CONFIG_ARCACHE__SHIFT: u32 = 12;
pub const VIVS_HI_AXI_CONFIG_ARCACHE__MASK: u32 = 0x0000_f000;

/// galcore: 0x00010, read to clear (`gckHARDWARE_Interrupt`).
pub const VIVS_HI_INTR_ACKNOWLEDGE: u32 = 0x0000_0010;
pub const VIVS_HI_INTR_ACKNOWLEDGE_INTR_VEC__MASK: u32 = 0x3fff_ffff;
pub const VIVS_HI_INTR_ACKNOWLEDGE_MMU_EXCEPTION: u32 = 0x4000_0000;
pub const VIVS_HI_INTR_ACKNOWLEDGE_AXI_BUS_ERROR: u32 = 0x8000_0000;
pub const VIVS_HI_INTR_ENBL: u32 = 0x0000_0014;

pub const VIVS_HI_CHIP_IDENTITY: u32 = 0x0000_0018;
pub const VIVS_HI_CHIP_FEATURE: u32 = 0x0000_001c;
pub const VIVS_HI_CHIP_MODEL: u32 = 0x0000_0020;
pub const VIVS_HI_CHIP_REV: u32 = 0x0000_0024;
pub const VIVS_HI_CHIP_DATE: u32 = 0x0000_0028;
pub const VIVS_HI_CHIP_TIME: u32 = 0x0000_002c;
pub const VIVS_HI_CHIP_CUSTOMER_ID: u32 = 0x0000_0030;
pub const VIVS_HI_CHIP_MINOR_FEATURE_0: u32 = 0x0000_0034;
pub const VIVS_HI_CHIP_MINOR_FEATURE_1: u32 = 0x0000_0074;
pub const VIVS_HI_CHIP_MINOR_FEATURE_2: u32 = 0x0000_0084;
pub const VIVS_HI_CHIP_MINOR_FEATURE_3: u32 = 0x0000_0088;
pub const VIVS_HI_CHIP_MINOR_FEATURE_4: u32 = 0x0000_0094;
pub const VIVS_HI_CHIP_MINOR_FEATURE_5: u32 = 0x0000_00a0;
pub const VIVS_HI_CHIP_PRODUCT_ID: u32 = 0x0000_00a8;
pub const VIVS_HI_CHIP_ECO_ID: u32 = 0x0000_00e8;

// ---- power management ----

pub const VIVS_PM_POWER_CONTROLS: u32 = 0x0000_0100;
pub const VIVS_PM_POWER_CONTROLS_ENABLE_MODULE_CLOCK_GATING: u32 = 0x0000_0001;
pub const VIVS_PM_MODULE_CONTROLS: u32 = 0x0000_0104;
pub const VIVS_PM_MODULE_CONTROLS_DISABLE_MODULE_CLOCK_GATING_RA_EZ: u32 = 0x0001_0000;
pub const VIVS_PM_MODULE_CONTROLS_DISABLE_MODULE_CLOCK_GATING_RA_HZ: u32 = 0x0002_0000;
pub const VIVS_PM_PULSE_EATER: u32 = 0x0000_010c;
pub const VIVS_PM_PULSE_EATER_DISABLE: u32 = 0x0000_0001;
pub const VIVS_PM_PULSE_EATER_UNK17: u32 = 0x0002_0000;
pub const VIVS_PM_PULSE_EATER_INTERNAL_DFS: u32 = 0x0004_0000;

// ---- MMU v2 ----

pub const VIVS_MMUv2_CONFIGURATION_MODE_MODE4_K: u32 = 0x0000_0000;
pub const VIVS_MMUv2_CONTROL: u32 = 0x0000_018c;
pub const VIVS_MMUv2_CONTROL_ENABLE: u32 = 0x0000_0001;
/// A state (written from the command stream), not reachable over the AHB.
pub const VIVS_MMUv2_PTA_CONFIG: u32 = 0x0000_01ac;
pub const VIVS_MMUv2_PTA_CONFIG_INDEX__MASK: u32 = 0x0000_ffff;
pub const VIVS_MMUv2_SEC_EXCEPTION_ADDR: u32 = 0x0000_0380;
pub const VIVS_MMUv2_SEC_STATUS: u32 = 0x0000_0384;
pub const VIVS_MMUv2_SEC_CONTROL: u32 = 0x0000_0388;
pub const VIVS_MMUv2_SEC_CONTROL_ENABLE: u32 = 0x0000_0001;
pub const VIVS_MMUv2_PTA_ADDRESS_LOW: u32 = 0x0000_038c;
pub const VIVS_MMUv2_PTA_ADDRESS_HIGH: u32 = 0x0000_0390;
pub const VIVS_MMUv2_PTA_CONTROL: u32 = 0x0000_0394;
pub const VIVS_MMUv2_PTA_CONTROL_ENABLE: u32 = 0x0000_0001;
pub const VIVS_MMUv2_NONSEC_SAFE_ADDR_LOW: u32 = 0x0000_0398;
pub const VIVS_MMUv2_SEC_SAFE_ADDR_LOW: u32 = 0x0000_039c;
pub const VIVS_MMUv2_SAFE_ADDRESS_CONFIG: u32 = 0x0000_03a0;
pub const VIVS_MMUv2_SAFE_ADDRESS_CONFIG_NON_SEC_SAFE_ADDR_HIGH__MASK: u32 = 0x0000_00ff;
pub const VIVS_MMUv2_SAFE_ADDRESS_CONFIG_SEC_SAFE_ADDR_HIGH__MASK: u32 = 0x00ff_0000;
pub const VIVS_MMUv2_SAFE_ADDRESS_CONFIG_SEC_SAFE_ADDR_HIGH__SHIFT: u32 = 16;
pub const VIVS_MMUv2_SEC_COMMAND_CONTROL: u32 = 0x0000_03a4;
pub const VIVS_MMUv2_SEC_COMMAND_CONTROL_PREFETCH__MASK: u32 = 0x0000_ffff;
pub const VIVS_MMUv2_SEC_COMMAND_CONTROL_ENABLE: u32 = 0x0001_0000;
pub const VIVS_MMUv2_AHB_CONTROL: u32 = 0x0000_03a8;
pub const VIVS_MMUv2_AHB_CONTROL_RESET: u32 = 0x0000_0001;
pub const VIVS_MMUv2_AHB_CONTROL_NONSEC_ACCESS: u32 = 0x0000_0002;

// ---- front end ----

pub const VIVS_FE_COMMAND_ADDRESS: u32 = 0x0000_0654;
pub const VIVS_FE_COMMAND_CONTROL: u32 = 0x0000_0658;
pub const VIVS_FE_COMMAND_CONTROL_PREFETCH__MASK: u32 = 0x0000_ffff;
pub const VIVS_FE_COMMAND_CONTROL_ENABLE: u32 = 0x0001_0000;
pub const VIVS_FE_DMA_DEBUG_STATE: u32 = 0x0000_0660;
pub const VIVS_FE_DMA_DEBUG_STATE_CMD_STATE__MASK: u32 = 0x0000_001f;
pub const VIVS_FE_DMA_DEBUG_STATE_CMD_DMA_STATE__MASK: u32 = 0x0000_0300;
pub const VIVS_FE_DMA_DEBUG_STATE_CMD_DMA_STATE__SHIFT: u32 = 8;
pub const VIVS_FE_DMA_ADDRESS: u32 = 0x0000_0664;
pub const VIVS_FE_DMA_LOW: u32 = 0x0000_0668;
pub const VIVS_FE_DMA_HIGH: u32 = 0x0000_066c;

// ---- 3D states the ring writes (state.xml.h; TS_FLUSH_CACHE state_3d.xml.h) ----

pub const VIVS_GL_PIPE_SELECT: u32 = 0x0000_3800;
pub const VIVS_GL_EVENT: u32 = 0x0000_3804;
pub const VIVS_GL_EVENT_EVENT_ID__MASK: u32 = 0x0000_001f;
pub const VIVS_GL_EVENT_FROM_FE: u32 = 0x0000_0020;
pub const VIVS_GL_EVENT_FROM_PE: u32 = 0x0000_0040;
pub const VIVS_GL_SEMAPHORE_TOKEN: u32 = 0x0000_3808;
pub const VIVS_GL_SEMAPHORE_TOKEN_FROM__MASK: u32 = 0x0000_001f;
pub const VIVS_GL_SEMAPHORE_TOKEN_TO__MASK: u32 = 0x0000_1f00;
pub const VIVS_GL_SEMAPHORE_TOKEN_TO__SHIFT: u32 = 8;
pub const VIVS_GL_FLUSH_CACHE: u32 = 0x0000_380c;
pub const VIVS_GL_FLUSH_CACHE_DEPTH: u32 = 0x0000_0001;
pub const VIVS_GL_FLUSH_CACHE_COLOR: u32 = 0x0000_0002;
pub const VIVS_GL_FLUSH_CACHE_TEXTURE: u32 = 0x0000_0004;
pub const VIVS_GL_FLUSH_CACHE_TEXTUREVS: u32 = 0x0000_0010;
pub const VIVS_GL_FLUSH_CACHE_SHADER_L1: u32 = 0x0000_0020;
pub const VIVS_TS_FLUSH_CACHE: u32 = 0x0000_1650;
pub const VIVS_TS_FLUSH_CACHE_FLUSH: u32 = 0x0000_0001;

// ---- command stream (cmdstream.xml.h) ----

pub const VIV_FE_LOAD_STATE_HEADER_OP_LOAD_STATE: u32 = 0x0800_0000;
pub const VIV_FE_LOAD_STATE_HEADER_FIXP: u32 = 0x0400_0000;
pub const VIV_FE_LOAD_STATE_HEADER_COUNT__MASK: u32 = 0x03ff_0000;
pub const VIV_FE_LOAD_STATE_HEADER_COUNT__SHIFT: u32 = 16;
pub const VIV_FE_LOAD_STATE_HEADER_OFFSET__MASK: u32 = 0x0000_ffff;
pub const VIV_FE_LOAD_STATE_HEADER_OFFSET__SHR: u32 = 2;
pub const VIV_FE_END_HEADER_OP_END: u32 = 0x1000_0000;
pub const VIV_FE_NOP_HEADER_OP_NOP: u32 = 0x1800_0000;
pub const VIV_FE_WAIT_HEADER_OP_WAIT: u32 = 0x3800_0000;
pub const VIV_FE_WAIT_HEADER_DELAY__MASK: u32 = 0x0000_ffff;
pub const VIV_FE_LINK_HEADER_OP_LINK: u32 = 0x4000_0000;
pub const VIV_FE_LINK_HEADER_PREFETCH__MASK: u32 = 0x0000_ffff;
pub const VIV_FE_STALL_HEADER_OP_STALL: u32 = 0x4800_0000;
pub const VIV_FE_STALL_TOKEN_FROM__MASK: u32 = 0x0000_001f;
pub const VIV_FE_STALL_TOKEN_TO__MASK: u32 = 0x0000_1f00;
pub const VIV_FE_STALL_TOKEN_TO__SHIFT: u32 = 8;

// ---- enums and feature bits (common.xml.h) ----

pub const PIPE_ID_PIPE_3D: u32 = 0x0000_0000;
pub const SYNC_RECIPIENT_FE: u32 = 0x0000_0001;
pub const SYNC_RECIPIENT_PE: u32 = 0x0000_0007;
pub const CHIP_MINOR_FEATURES0_MORE_MINOR_FEATURES: u32 = 0x0020_0000;
pub const CHIP_MINOR_FEATURES1_MMU_VERSION: u32 = 0x1000_0000;
pub const CHIP_MINOR_FEATURES7_BIT_SECURITY: u32 = 0x0020_0000;
pub const CHIP_MINOR_FEATURES10_SECURITY_AHB: u32 = 0x0000_4000;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const HEADERS: [&str; 5] = [
        include_str!("../../../../src/vivante/hw/state_3d.xml.h"),
        include_str!("../../../../src/vivante/hw/state_hi.xml.h"),
        include_str!("../../../../src/vivante/hw/state.xml.h"),
        include_str!("../../../../src/vivante/hw/cmdstream.xml.h"),
        include_str!("../../../../src/vivante/hw/common.xml.h"),
    ];

    fn defines() -> HashMap<String, u32> {
        let mut m = HashMap::new();
        for h in HEADERS {
            for l in h.lines() {
                let mut w = l.split_whitespace();
                if w.next() != Some("#define") {
                    continue;
                }
                let (Some(name), Some(val)) = (w.next(), w.next()) else { continue };
                let v = if let Some(hex) = val.strip_prefix("0x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    val.parse().ok()
                };
                if let Some(v) = v {
                    m.insert(name.to_string(), v);
                }
            }
        }
        m
    }

    /// Every constant above is the rnndb header's, by name.
    #[test]
    fn constants_are_the_rnndb_headers() {
        let d = defines();
        let check = |name: &str, v: u32| {
            let want = d.get(name).unwrap_or_else(|| panic!("{name} is not in hw/*.xml.h"));
            assert_eq!(v, *want, "{name}");
        };
        macro_rules! same {
            ($($n:ident),* $(,)?) => { $( check(stringify!($n), $n); )* };
        }
        same!(
            VIVS_HI_CLOCK_CONTROL,
            VIVS_HI_CLOCK_CONTROL_FSCALE_VAL__MASK,
            VIVS_HI_CLOCK_CONTROL_FSCALE_VAL__SHIFT,
            VIVS_HI_CLOCK_CONTROL_FSCALE_CMD_LOAD,
            VIVS_HI_CLOCK_CONTROL_DISABLE_DEBUG_REGISTERS,
            VIVS_HI_CLOCK_CONTROL_SOFT_RESET,
            VIVS_HI_CLOCK_CONTROL_IDLE_3D,
            VIVS_HI_CLOCK_CONTROL_IDLE_2D,
            VIVS_HI_CLOCK_CONTROL_ISOLATE_GPU,
            VIVS_HI_IDLE_STATE,
            VIVS_HI_IDLE_STATE_FE,
            VIVS_HI_IDLE_STATE_DE,
            VIVS_HI_IDLE_STATE_PE,
            VIVS_HI_IDLE_STATE_SH,
            VIVS_HI_IDLE_STATE_PA,
            VIVS_HI_IDLE_STATE_SE,
            VIVS_HI_IDLE_STATE_RA,
            VIVS_HI_IDLE_STATE_TX,
            VIVS_HI_IDLE_STATE_MC,
            VIVS_HI_IDLE_STATE_AXI_LP,
            VIVS_HI_AXI_CONFIG,
            VIVS_HI_AXI_CONFIG_AWCACHE__SHIFT,
            VIVS_HI_AXI_CONFIG_AWCACHE__MASK,
            VIVS_HI_AXI_CONFIG_ARCACHE__SHIFT,
            VIVS_HI_AXI_CONFIG_ARCACHE__MASK,
            VIVS_HI_INTR_ACKNOWLEDGE,
            VIVS_HI_INTR_ACKNOWLEDGE_INTR_VEC__MASK,
            VIVS_HI_INTR_ACKNOWLEDGE_MMU_EXCEPTION,
            VIVS_HI_INTR_ACKNOWLEDGE_AXI_BUS_ERROR,
            VIVS_HI_INTR_ENBL,
            VIVS_HI_CHIP_IDENTITY,
            VIVS_HI_CHIP_FEATURE,
            VIVS_HI_CHIP_MODEL,
            VIVS_HI_CHIP_REV,
            VIVS_HI_CHIP_DATE,
            VIVS_HI_CHIP_TIME,
            VIVS_HI_CHIP_CUSTOMER_ID,
            VIVS_HI_CHIP_MINOR_FEATURE_0,
            VIVS_HI_CHIP_MINOR_FEATURE_1,
            VIVS_HI_CHIP_MINOR_FEATURE_2,
            VIVS_HI_CHIP_MINOR_FEATURE_3,
            VIVS_HI_CHIP_MINOR_FEATURE_4,
            VIVS_HI_CHIP_MINOR_FEATURE_5,
            VIVS_HI_CHIP_PRODUCT_ID,
            VIVS_HI_CHIP_ECO_ID,
            VIVS_PM_POWER_CONTROLS,
            VIVS_PM_POWER_CONTROLS_ENABLE_MODULE_CLOCK_GATING,
            VIVS_PM_MODULE_CONTROLS,
            VIVS_PM_MODULE_CONTROLS_DISABLE_MODULE_CLOCK_GATING_RA_EZ,
            VIVS_PM_MODULE_CONTROLS_DISABLE_MODULE_CLOCK_GATING_RA_HZ,
            VIVS_PM_PULSE_EATER,
            VIVS_PM_PULSE_EATER_DISABLE,
            VIVS_PM_PULSE_EATER_UNK17,
            VIVS_PM_PULSE_EATER_INTERNAL_DFS,
            VIVS_MMUv2_CONFIGURATION_MODE_MODE4_K,
            VIVS_MMUv2_CONTROL,
            VIVS_MMUv2_CONTROL_ENABLE,
            VIVS_MMUv2_PTA_CONFIG,
            VIVS_MMUv2_PTA_CONFIG_INDEX__MASK,
            VIVS_MMUv2_SEC_EXCEPTION_ADDR,
            VIVS_MMUv2_SEC_STATUS,
            VIVS_MMUv2_SEC_CONTROL,
            VIVS_MMUv2_SEC_CONTROL_ENABLE,
            VIVS_MMUv2_PTA_ADDRESS_LOW,
            VIVS_MMUv2_PTA_ADDRESS_HIGH,
            VIVS_MMUv2_PTA_CONTROL,
            VIVS_MMUv2_PTA_CONTROL_ENABLE,
            VIVS_MMUv2_NONSEC_SAFE_ADDR_LOW,
            VIVS_MMUv2_SEC_SAFE_ADDR_LOW,
            VIVS_MMUv2_SAFE_ADDRESS_CONFIG,
            VIVS_MMUv2_SAFE_ADDRESS_CONFIG_NON_SEC_SAFE_ADDR_HIGH__MASK,
            VIVS_MMUv2_SAFE_ADDRESS_CONFIG_SEC_SAFE_ADDR_HIGH__MASK,
            VIVS_MMUv2_SAFE_ADDRESS_CONFIG_SEC_SAFE_ADDR_HIGH__SHIFT,
            VIVS_MMUv2_SEC_COMMAND_CONTROL,
            VIVS_MMUv2_SEC_COMMAND_CONTROL_PREFETCH__MASK,
            VIVS_MMUv2_SEC_COMMAND_CONTROL_ENABLE,
            VIVS_MMUv2_AHB_CONTROL,
            VIVS_MMUv2_AHB_CONTROL_RESET,
            VIVS_MMUv2_AHB_CONTROL_NONSEC_ACCESS,
            VIVS_FE_COMMAND_ADDRESS,
            VIVS_FE_COMMAND_CONTROL,
            VIVS_FE_COMMAND_CONTROL_PREFETCH__MASK,
            VIVS_FE_COMMAND_CONTROL_ENABLE,
            VIVS_FE_DMA_DEBUG_STATE,
            VIVS_FE_DMA_DEBUG_STATE_CMD_STATE__MASK,
            VIVS_FE_DMA_DEBUG_STATE_CMD_DMA_STATE__MASK,
            VIVS_FE_DMA_DEBUG_STATE_CMD_DMA_STATE__SHIFT,
            VIVS_FE_DMA_ADDRESS,
            VIVS_FE_DMA_LOW,
            VIVS_FE_DMA_HIGH,
            VIVS_GL_PIPE_SELECT,
            VIVS_GL_EVENT,
            VIVS_GL_EVENT_EVENT_ID__MASK,
            VIVS_GL_EVENT_FROM_FE,
            VIVS_GL_EVENT_FROM_PE,
            VIVS_GL_SEMAPHORE_TOKEN,
            VIVS_GL_SEMAPHORE_TOKEN_FROM__MASK,
            VIVS_GL_SEMAPHORE_TOKEN_TO__MASK,
            VIVS_GL_SEMAPHORE_TOKEN_TO__SHIFT,
            VIVS_GL_FLUSH_CACHE,
            VIVS_GL_FLUSH_CACHE_DEPTH,
            VIVS_GL_FLUSH_CACHE_COLOR,
            VIVS_GL_FLUSH_CACHE_TEXTURE,
            VIVS_GL_FLUSH_CACHE_TEXTUREVS,
            VIVS_GL_FLUSH_CACHE_SHADER_L1,
            VIVS_TS_FLUSH_CACHE,
            VIVS_TS_FLUSH_CACHE_FLUSH,
            VIV_FE_LOAD_STATE_HEADER_OP_LOAD_STATE,
            VIV_FE_LOAD_STATE_HEADER_FIXP,
            VIV_FE_LOAD_STATE_HEADER_COUNT__MASK,
            VIV_FE_LOAD_STATE_HEADER_COUNT__SHIFT,
            VIV_FE_LOAD_STATE_HEADER_OFFSET__MASK,
            VIV_FE_LOAD_STATE_HEADER_OFFSET__SHR,
            VIV_FE_END_HEADER_OP_END,
            VIV_FE_NOP_HEADER_OP_NOP,
            VIV_FE_WAIT_HEADER_OP_WAIT,
            VIV_FE_WAIT_HEADER_DELAY__MASK,
            VIV_FE_LINK_HEADER_OP_LINK,
            VIV_FE_LINK_HEADER_PREFETCH__MASK,
            VIV_FE_STALL_HEADER_OP_STALL,
            VIV_FE_STALL_TOKEN_FROM__MASK,
            VIV_FE_STALL_TOKEN_TO__MASK,
            VIV_FE_STALL_TOKEN_TO__SHIFT,
            PIPE_ID_PIPE_3D,
            SYNC_RECIPIENT_FE,
            SYNC_RECIPIENT_PE,
        );
        // rnndb spells the feature words in lower camel case
        check("chipMinorFeatures0_MORE_MINOR_FEATURES", CHIP_MINOR_FEATURES0_MORE_MINOR_FEATURES);
        check("chipMinorFeatures1_MMU_VERSION", CHIP_MINOR_FEATURES1_MMU_VERSION);
        check("chipMinorFeatures7_BIT_SECURITY", CHIP_MINOR_FEATURES7_BIT_SECURITY);
        check("chipMinorFeatures10_SECURITY_AHB", CHIP_MINOR_FEATURES10_SECURITY_AHB);
    }
}
