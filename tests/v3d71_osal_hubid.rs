// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! OSAL_QUERY_V3D_HUB_IDENT (finding F1 of docs/notes/v3d71-table71.md): the
//! Rust constant in src/osal/mod.rs, the C define in src/osal/osal_internal.h
//! and the case in the 7.1 Zephyr OSAL agree on the number, and the C case
//! checks an 8-byte out and writes HUB_IDENT1 then HUB_IDENT3. The number is
//! also free of every other query's.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_osal_hubid`

const MOD_RS: &str = include_str!("../src/osal/mod.rs");
const INTERNAL_H: &str = include_str!("../src/osal/osal_internal.h");
const V3D71_C: &str = include_str!("../src/osal/zephyr_v3d71.c");

fn rust_const(name: &str) -> u32 {
    let key = format!("pub const {name}: u32 = ");
    let at = MOD_RS.find(&key).unwrap_or_else(|| panic!("{name} missing")) + key.len();
    let rest = &MOD_RS[at..];
    rest[..rest.find(';').unwrap()].trim().parse().unwrap()
}

#[test]
fn number_matches_between_rust_and_c() {
    let n = rust_const("OSAL_QUERY_V3D_HUB_IDENT");
    let key = "#define OSAL_QUERY_V3D_HUB_IDENT ";
    let at = INTERNAL_H.find(key).expect("C define") + key.len();
    let c: u32 = INTERNAL_H[at..].split_whitespace().next().unwrap().trim_end_matches('u').parse().unwrap();
    assert_eq!(n, c);
    assert_eq!(n, 20);
}

#[test]
fn number_is_unique_among_rust_queries() {
    let n = rust_const("OSAL_QUERY_V3D_HUB_IDENT");
    let same = MOD_RS
        .lines()
        .filter(|l| l.starts_with("pub const OSAL_QUERY_") && l.ends_with(&format!(": u32 = {n};")))
        .count();
    assert_eq!(same, 1);
}

#[test]
fn c_case_writes_ident1_then_ident3() {
    let at = V3D71_C.find("what == OSAL_QUERY_V3D_HUB_IDENT").expect("case in zephyr_v3d71.c");
    let body = &V3D71_C[at..];
    let body = &body[..body.find("\n\t}\n").unwrap()];
    assert!(body.contains("out_sz == 2 * sizeof(uint32_t)"), "{body}");
    assert!(body.contains("[0] = s.id.hub_ident[1]"), "{body}");
    assert!(body.contains("[1] = s.id.hub_ident[3]"), "{body}");
    assert_eq!(body.matches("] = ").count(), 2, "exactly two u32 written");
}
