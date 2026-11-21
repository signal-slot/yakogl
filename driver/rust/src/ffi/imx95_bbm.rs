// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95's record sink (driver/zephyr/lib/board/verdin_imx95/
//! verdin_imx95_record.c): [`yd_imx95_bbm_record_store`] puts the board
//! layer's four record words into the BBNSM GPR4..7 over the board's SCMI
//! channel ([`crate::soc::imx95::bbm`]), [`yd_imx95_bbm_record_load`] reads
//! them back. No env, no log: the caller prints what failed from
//! [`YdImx95Bbm`].

use super::imx95_display::CScmi;
use crate::errno::{EINVAL, EIO};
use crate::firmware::scmi::Transport;
use crate::soc::imx95::bbm::{self, RECORD_WORDS};
use crate::soc::imx95::sm::Fault;
use core::mem::{offset_of, size_of};

/// `struct yd_imx95_bbm`: where a store or a load stopped (all 0 when it
/// did not).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImx95Bbm {
    /// The GPR whose message failed.
    pub gpr: u32,
    /// BBM_GPR_SET (3) or BBM_GPR_GET (4).
    pub msg: u32,
    /// The transport's errno (negative), else 0.
    pub rc: i32,
    /// The SM's SCMI status of a refusal (negative), else 0.
    pub status: i32,
    /// 1 when the reply did not answer the command (its header or length).
    pub malformed: u32,
}

const _: () = assert!(size_of::<YdImx95Bbm>() == 20);
const _: () = assert!(offset_of!(YdImx95Bbm, rc) == 8);
const _: () = assert!(offset_of!(YdImx95Bbm, malformed) == 16);

impl From<bbm::Error> for YdImx95Bbm {
    fn from(e: bbm::Error) -> YdImx95Bbm {
        let mut o = YdImx95Bbm { gpr: e.gpr, msg: e.msg as u32, ..YdImx95Bbm::default() };
        match e.fault {
            Fault::Transport(rc) => o.rc = rc,
            Fault::Refused(s) => o.status = s.code(),
            Fault::Echo { .. } | Fault::Length(_) => o.malformed = 1,
        }
        o
    }
}

/// [`yd_imx95_bbm_record_store`] over any transport.
pub fn store_on<T: Transport>(t: &mut T, w: &[u32; RECORD_WORDS], out: &mut YdImx95Bbm) -> i32 {
    *out = YdImx95Bbm::default();
    match bbm::store(t, w) {
        Ok(()) => 0,
        Err(e) => {
            *out = e.into();
            -EIO
        }
    }
}

/// [`yd_imx95_bbm_record_load`] over any transport.
pub fn load_on<T: Transport>(t: &mut T, w: &mut [u32; RECORD_WORDS], out: &mut YdImx95Bbm) -> i32 {
    *out = YdImx95Bbm::default();
    match bbm::load(t) {
        Ok(v) => {
            *w = v;
            0
        }
        Err(e) => {
            *out = e.into();
            -EIO
        }
    }
}

/// Store the record `w` (four words, w0 the magic) in GPR4..7 through
/// `scmi`: the magic's GPR cleared first, then w1..w3, then w0. 0 when every
/// message was answered with SUCCESS; `-EIO` when one was not (`out` says
/// which and why); `-EINVAL` for a null argument (nothing sent).
///
/// # Safety
/// `w` valid for four reads, `out` for a write; `scmi` safe to call with
/// valid buffers (in the context the caller is in: it may be called with
/// interrupts locked when `scmi` polls).
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_bbm_record_store(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    w: *const u32,
    out: *mut YdImx95Bbm,
) -> i32 {
    let Some(scmi) = scmi else { return -EINVAL };
    if w.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract: four words at w, out writable.
    let (w, out) = unsafe { (&*(w as *const [u32; RECORD_WORDS]), &mut *out) };
    store_on(&mut CScmi(scmi), w, out)
}

/// Read GPR4..7 into `w` (w0..w3) through `scmi`. 0, `-EIO` (`out` says
/// which and why; `w` untouched), `-EINVAL` for a null argument.
///
/// # Safety
/// `w` valid for four writes, `out` for a write; `scmi` as for
/// [`yd_imx95_bbm_record_store`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_bbm_record_load(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    w: *mut u32,
    out: *mut YdImx95Bbm,
) -> i32 {
    let Some(scmi) = scmi else { return -EINVAL };
    if w.is_null() || out.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (w, out) = unsafe { (&mut *(w as *mut [u32; RECORD_WORDS]), &mut *out) };
    load_on(&mut CScmi(scmi), w, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::imx95::sm::Status;
    use crate::soc::imx95::sm_mock::{Sm, PERM_NONE};

    #[test]
    fn store_then_load_over_the_mock_sm() {
        let mut sm = Sm::verdin();
        let w = [0x594b_0201, 1, 2, 3];
        let mut o = YdImx95Bbm { gpr: 9, ..Default::default() };
        assert_eq!(store_on(&mut sm, &w, &mut o), 0);
        assert_eq!(o, YdImx95Bbm::default());
        let mut got = [0u32; 4];
        assert_eq!(load_on(&mut sm, &mut got, &mut o), 0);
        assert_eq!(got, w);
    }

    #[test]
    fn a_refusal_names_the_gpr_message_and_status() {
        let mut sm = Sm::verdin();
        sm.gpr_perm.insert(7, PERM_NONE);
        let mut o = YdImx95Bbm::default();
        assert_eq!(store_on(&mut sm, &[0x594b_0201, 1, 2, 3], &mut o), -EIO);
        assert_eq!(o, YdImx95Bbm { gpr: 7, msg: 3, rc: 0, status: Status::Denied.code(), malformed: 0 });
        let mut got = [5u32; 4];
        assert_eq!(load_on(&mut sm, &mut got, &mut o), -EIO);
        assert_eq!((o.gpr, o.msg), (7, 4));
        assert_eq!(got, [5; 4]);
    }

    #[test]
    fn null_arguments_send_nothing() {
        extern "C" fn never(_: *const u8, _: usize, _: *mut u8, _: usize) -> i32 {
            panic!("nothing may be sent")
        }
        let mut o = YdImx95Bbm::default();
        let w = [0u32; 4];
        // SAFETY: null pointers are what is tested; nothing is dereferenced.
        unsafe {
            assert_eq!(yd_imx95_bbm_record_store(None, w.as_ptr(), &mut o), -EINVAL);
            assert_eq!(yd_imx95_bbm_record_store(Some(never), core::ptr::null(), &mut o), -EINVAL);
            assert_eq!(yd_imx95_bbm_record_load(Some(never), core::ptr::null_mut(), &mut o), -EINVAL);
        }
    }
}
