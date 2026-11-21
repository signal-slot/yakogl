// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the Vivante arm of [`crate::backend::storage_current`],
//! W10's groundwork (docs/notes/gc-tiled-wire.md W10, "CPU readers").
//!
//! Under W10 a single-level 32-bit target that is sampled from its TILED
//! shadow keeps no RS write-back in the stream that renders it: the shadow is
//! the newer copy after the kick ([`wide_target::State::RsNewer`]). Before
//! the GL core reads or writes that texture's texels on the CPU it calls
//! `storage_current`, and this arm records the RS tiled to linear copy
//! ([`tiled_rt::untile`]'s job, the shape W4's frame-end write-back takes),
//! waits for every kick, and leaves the storage the newer copy.
//!
//! Nothing marks an entry `RsNewer` yet (`wide_target::mark_rs_newer` has
//! no caller outside the tests; sampling from the shadow is item
//! w10-sample-knob, behind `gc_tiled_sample`), so today
//! [`gc_storage_current`] finds no entry and does nothing: every stream and
//! every ES 2.0 answer is unchanged.

use super::backend::{self, RsBlit};
use super::cmd::GcCmd;
use super::tiled_rt::{self, Spec};
use super::wide_target::{self, State};
use crate::backend::GlesTextureBind;

/// The RS job that makes the storage of `spec` current from its shadow at
/// `shadow`, when the shadow is in `state` -- only [`State::RsNewer`] needs
/// one: [`tiled_rt::untile`]'s tiled to linear copy. Every other state has
/// the storage current already, or has a write-back of its own in the
/// stream (`InStep`) or in [`wide_target::retire`] (`ShadowNewer`).
pub(super) fn resolve_of(state: State, spec: &Spec, shadow: u64) -> Option<RsBlit> {
    match state {
        State::RsNewer(_) => Some(tiled_rt::untile(spec, shadow)),
        State::InStep | State::ShadowNewer(_) | State::StorageNewer => None,
    }
}

/// [`resolve_of`] recorded into `c` (the RS block of `backend::emit_rs`,
/// flushes and the PE drain first): true when a job was recorded.
pub fn record_resolve(c: &mut GcCmd, state: State, spec: &Spec, shadow: u64) -> bool {
    match resolve_of(state, spec, shadow) {
        Some(b) => {
            backend::emit_rs(c, &b);
            true
        }
        None => false,
    }
}

/// The GL core is about to touch the texels of `b` on the CPU: a lazy
/// shadow of its storage ([`State::RsNewer`]) is written back by the RS and
/// every kick waited for, and the storage is the newer copy from then on.
/// No shadow, or one that needs no write-back: nothing recorded, no wait.
pub fn gc_storage_current(b: &GlesTextureBind) {
    let Some((spec, shadow)) = wide_target::rs_newer(b.base_gpu) else { return };
    let Some(job) = resolve_of(State::RsNewer(0), &spec, shadow) else { return };
    if backend::rs_resolve_and_finish(&job) != 0 {
        crate::glog_once!("yakogl/vivante: the RS write-back of a tiled shadow before a CPU read failed");
        return;
    }
    wide_target::mark_storage_newer(b.base_gpu);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec { gpu: 0x4800_0000, w: 256, h: 256, pitch: 1024 };
    const SHADOW: u64 = 0x4900_0000;

    #[test]
    fn only_a_lazy_shadow_gets_a_resolve_and_it_is_untile() {
        let want = tiled_rt::untile(&SPEC, SHADOW);
        let got = resolve_of(State::RsNewer(7), &SPEC, SHADOW).expect("a lazy shadow resolves");
        assert_eq!(
            (got.config, got.src, got.src_stride, got.dst, got.dst_stride, got.w, got.h),
            (want.config, want.src, want.src_stride, want.dst, want.dst_stride, want.w, want.h)
        );
        for s in [State::InStep, State::ShadowNewer(7), State::StorageNewer] {
            assert!(resolve_of(s, &SPEC, SHADOW).is_none(), "{s:?}");
        }
    }

    #[test]
    fn no_shadow_is_a_no_op() {
        /* no entry at this address: returns before the backend is asked
         * (tests/gc_storage_current.rs counts the finishes in a process of
         * its own) */
        assert_eq!(wide_target::rs_newer(0x4a00_0000), None);
        gc_storage_current(&GlesTextureBind { base_gpu: 0x4a00_0000, width: 16, height: 4, ..GlesTextureBind::ZERO });
    }
}
