// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Host test, no feature requirement: every cargo feature a C build path
// passes to `cargo build -p yakogl-capi --features ...` is forwarded by
// capi/Cargo.toml as `name = ["yakogl/name"]`.
//
// Features: host-test,gpu-powervr,shaderc-online,no-shader-table (it only reads files).

use std::collections::BTreeSet;
use std::fs;

fn read(p: &str) -> String {
    fs::read_to_string(format!("{}/{}", env!("CARGO_MANIFEST_DIR"), p)).unwrap()
}

/// Feature names of capi/Cargo.toml's [features] with their values.
fn forwarded() -> Vec<(String, String)> {
    let toml = read("capi/Cargo.toml");
    let mut out = Vec::new();
    let mut in_features = false;
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_features = l == "[features]";
            continue;
        }
        if !in_features || l.starts_with('#') || l.is_empty() {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn passed() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    // capi/yakogl_capi.cmake: list(APPEND _features "name") with a literal.
    for line in read("capi/yakogl_capi.cmake").lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("list(APPEND _features \"") {
            if let Some(name) = rest.split('"').next() {
                if !name.contains('$') {
                    set.insert(name.to_string());
                }
            }
        }
    }
    // baremetal.mk: CARGO_FEATURES := a,b,$(CARGO_FEATURES),c
    for line in read("baremetal.mk").lines() {
        if let Some(rest) = line.trim().strip_prefix("CARGO_FEATURES :=") {
            for n in rest.split(',') {
                let n = n.trim();
                if !n.is_empty() && !n.contains('$') {
                    set.insert(n.to_string());
                }
            }
        }
    }
    set
}

#[test]
fn every_passed_feature_is_forwarded() {
    let fwd = forwarded();
    let passed = passed();
    assert!(passed.len() >= 8, "parsed too few features: {passed:?}");
    for name in &passed {
        let (_, v) =
            fwd.iter().find(|(k, _)| k == name).unwrap_or_else(|| panic!("capi/Cargo.toml does not forward `{name}`"));
        if v.contains("dep:") {
            continue; // forwards to another crate on purpose
        }
        assert!(v.contains(&format!("\"yakogl/{name}\"")), "`{name}` is not forwarded as yakogl/{name}: {v}");
    }
}

#[test]
fn opt_in_es3_features_are_forwarded() {
    let fwd = forwarded();
    for name in ["msaa-fbo", "color-buffer-float", "pvr-instance-divisor"] {
        assert!(fwd.iter().any(|(k, _)| k == name), "{name} missing");
    }
}
