// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::bbm`] against the mock System Manager ([`super::sm_mock`]): the
//! record's four words in GPR4..7 over the BBM protocol, in the order the
//! record's rule asks (the magic cleared first, written last), refused as
//! the SM refuses (GPR0..3 are not AP-NS's), and read back as the same words
//! the console line carries.

use super::bbm::{self, BBM_GPR_GET, BBM_GPR_SET, PROTO_BBM, RECORD_GPR};
use super::sm::{Fault, Status};
use super::sm_mock::{req, Sm, B, PERM_NONE};
use crate::board::record::{self, kind, Class};
use crate::firmware::scmi::{self, Transport};

/// An APP pass record as the board layer's writer makes it (the first
/// result of a run: sequence 1).
fn app_pass() -> [u32; 4] {
    let mut w = record::Writer::new();
    w.result(kind::APP, 1, 120, 7)
}

#[test]
fn the_ids_are_nxps_and_the_headers_the_linux_reader_sends() {
    // Linux include/linux/scmi_imx_protocol.h SCMI_PROTOCOL_IMX_BBM 0x81;
    // imx-sm-bbm.c IMX_BBM_GPR_SET 0x3, IMX_BBM_GPR_GET 0x4; the SM's
    // rpc_scmi_bbm.c COMMAND_BBM_GPR_SET/GET the same.
    assert_eq!((PROTO_BBM, BBM_GPR_SET, BBM_GPR_GET), (0x81, 0x3, 0x4));
    assert_eq!((bbm::NUM_GPR, RECORD_GPR), (8, 4));
    // tools/imx95_bbm_record.sh builds these words itself (token 0x10 + the
    // GPR): its selftest pins the same values
    assert_eq!(bbm::header(BBM_GPR_GET, 0), 0x0002_0404);
    assert_eq!(bbm::header(BBM_GPR_SET, 0), 0x0002_0403);
    assert_eq!(bbm::header(BBM_GPR_GET, 0x14), 0x0052_0404);
    assert_eq!(bbm::header(BBM_GPR_SET, 0x17), 0x005e_0403);
}

#[test]
fn store_clears_the_magic_then_writes_w1_to_w3_then_w0() {
    let w = app_pass();
    let mut sm = Sm::verdin();
    sm.gpr[4..8].copy_from_slice(&[0x594b_0201, 0xdead, 0xbeef, 0xcafe]); // an older record
    bbm::store(&mut sm, &w).unwrap();
    assert_eq!(
        sm.log,
        vec![
            req(B, BBM_GPR_SET, &[4, 0]),
            req(B, BBM_GPR_SET, &[5, w[1]]),
            req(B, BBM_GPR_SET, &[6, w[2]]),
            req(B, BBM_GPR_SET, &[7, w[3]]),
            req(B, BBM_GPR_SET, &[4, w[0]]),
        ]
    );
    assert_eq!(sm.gpr[4..8], w);
    // GPR0..3 (the SM's shutdown record) untouched
    assert_eq!(sm.gpr[..4], [0; 4]);
}

#[test]
fn load_reads_gpr4_to_7_and_gives_the_console_lines_words() {
    let w = app_pass();
    let mut sm = Sm::verdin();
    bbm::store(&mut sm, &w).unwrap();
    sm.log.clear();
    let got = bbm::load(&mut sm).unwrap();
    assert_eq!(sm.log, (4..8).map(|g| req(B, BBM_GPR_GET, &[g])).collect::<Vec<_>>());
    assert_eq!(got, w);
    // the same record the image prints as its yk-record line, and a verdict
    let mut line = String::new();
    record::write_line(&mut line, w).unwrap();
    assert_eq!(record::parse_line(&line), Some(got));
    assert_eq!(record::classify(got), Class::Verdict);
    assert_eq!(got[0], 0x594b_0201);
}

#[test]
fn a_breadcrumb_then_a_result_leaves_the_result() {
    // the WDOG3 arming's breadcrumb is stored early, the run's verdict later:
    // the GPRs hold the last record, as the SNVS does on the i.MX8MP
    let mut wr = record::Writer::new();
    let mut sm = Sm::verdin();
    let crumb = wr.progress(record::WHERE_KERNEL, 12).unwrap();
    bbm::store(&mut sm, &crumb).unwrap();
    assert!(matches!(record::classify(bbm::load(&mut sm).unwrap()), Class::DiedAfter(_)));
    let res = wr.result(kind::APP, 1, 3, 0);
    bbm::store(&mut sm, &res).unwrap();
    assert_eq!(bbm::load(&mut sm).unwrap(), res);
}

#[test]
fn an_agent_without_the_gprs_is_denied_and_nothing_changes() {
    // a configuration that does not give AP-NS the GPRs (GPR0..3 on
    // Toradex's: the SM's own)
    let mut sm = Sm::verdin();
    for g in 4..8 {
        sm.gpr_perm.insert(g, PERM_NONE);
    }
    sm.gpr[4] = 0x594b_0203;
    let e = bbm::store(&mut sm, &app_pass()).unwrap_err();
    assert_eq!(e, bbm::Error { gpr: 4, msg: BBM_GPR_SET, fault: Fault::Refused(Status::Denied) });
    assert_eq!(sm.log.len(), 1);
    assert_eq!(sm.gpr[4], 0x594b_0203);
    let e = bbm::load(&mut sm).unwrap_err();
    assert_eq!(e.fault, Fault::Refused(Status::Denied));
}

#[test]
fn a_store_cut_off_half_way_never_shows_the_new_magic() {
    let mut sm = Sm::verdin();
    sm.gpr[4..8].copy_from_slice(&[0x594b_0207, 1, 2, 3]);
    sm.gpr_perm.insert(6, PERM_NONE);
    let w = app_pass();
    let e = bbm::store(&mut sm, &w).unwrap_err();
    assert_eq!((e.gpr, e.msg), (6, BBM_GPR_SET));
    // the magic was cleared before the words: a reader sees no record
    assert_eq!(sm.gpr[4], 0);
    assert_eq!(record::classify([sm.gpr[4], sm.gpr[5], sm.gpr[6], sm.gpr[7]]), Class::NoMagic);
}

#[test]
fn the_sm_counts_eight_gprs_and_refuses_an_index_past_them() {
    let mut sm = Sm::verdin();
    let r = super::sm::call(&mut sm, PROTO_BBM, scmi::PROTOCOL_ATTRIBUTES, &[], 1).unwrap();
    assert_eq!(r.get(0) & 0xffff, bbm::NUM_GPR);
    let e = super::sm::call(&mut sm, PROTO_BBM, BBM_GPR_GET, &[8], 1).map(|_| ()).unwrap_err();
    assert_eq!(e, Fault::Refused(Status::NotFound));
}

#[test]
fn a_failing_transport_names_the_gpr() {
    struct Dead;
    impl Transport for Dead {
        fn transfer(&mut self, _: &[u8], _: &mut [u8]) -> Result<usize, i32> {
            Err(-19)
        }
    }
    let e = bbm::store(&mut Dead, &app_pass()).unwrap_err();
    assert_eq!(e, bbm::Error { gpr: 4, msg: BBM_GPR_SET, fault: Fault::Transport(-19) });
}
