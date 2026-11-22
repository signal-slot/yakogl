// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every cargo feature a C build path passes to `-p yakogl-capi` must be a
//! key of capi/Cargo.toml's `[features]` table: cargo refuses an unknown
//! feature, or (through a variable) quietly never sets it.
//!
//! Feature sets to run it (the test itself needs none, but the yakogl crate it
//! is built with does): `cargo test --test capi_feature_forward --features
//! host-test,gpu-powervr,shaderc-online`.
//! The sources are scanned as text: capi/yakogl_capi.cmake (the
//! `list(APPEND _features "...")` lines and the `osal-`/`gpu-` prefixes),
//! baremetal.mk (`CARGO_FEATURES` lists) and zephyr/CMakeLists.txt (the
//! crate features its comments name as "the crate's `x` feature"; the
//! archive itself is built by yakogl_add_capi_archive).

use std::collections::BTreeSet;
use std::fs;

fn read(path: &str) -> String {
    fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The keys of capi/Cargo.toml's [features] table.
fn capi_features() -> BTreeSet<String> {
    let text = read("capi/Cargo.toml");
    let mut in_features = false;
    let mut keys = BTreeSet::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_features = l == "[features]";
        } else if in_features && !l.starts_with('#') {
            if let Some((k, _)) = l.split_once('=') {
                keys.insert(k.trim().to_string());
            }
        }
    }
    keys
}

fn is_feature_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn cmake_features() -> BTreeSet<String> {
    let text = read("capi/yakogl_capi.cmake");
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("list(APPEND _features \"") else {
            continue;
        };
        let name = rest.split('"').next().unwrap();
        if let Some(prefix) = name.strip_suffix("${A_OSAL}") {
            assert_eq!(prefix, "osal-");
            out.insert("osal-linux".into());
            out.insert("osal-zephyr".into());
            out.insert("osal-baremetal".into());
        } else if name == "gpu-${A_GPU}" {
            for g in ["powervr", "vivante", "v3d", "mali"] {
                out.insert(format!("gpu-{g}"));
            }
        } else {
            out.insert(name.to_string());
        }
    }
    out
}

fn baremetal_features() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in read("baremetal.mk").lines() {
        let Some((lhs, rhs)) = line.split_once(":=") else {
            continue;
        };
        if lhs.trim() != "CARGO_FEATURES" {
            continue;
        }
        for f in rhs.split(',') {
            let f = f.trim();
            if f != "$(CARGO_FEATURES)" {
                out.insert(f.to_string());
            }
        }
    }
    out
}

fn zephyr_features() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in read("zephyr/CMakeLists.txt").lines() {
        // "# ... (the crate's `name` feature" -- a feature this file's
        // Kconfig mapping hands to yakogl_add_capi_archive().
        if let Some(i) = line.find("the crate's `") {
            let rest = &line[i + "the crate's `".len()..];
            if let Some((name, tail)) = rest.split_once('`') {
                if tail.trim_start().starts_with("feature") && is_feature_name(name) {
                    out.insert(name.to_string());
                }
            }
        }
    }
    out
}

fn check(path: &str, passed: BTreeSet<String>, forwarded: &BTreeSet<String>) {
    assert!(!passed.is_empty(), "{path}: no feature names found (scan broke?)");
    for f in &passed {
        assert!(
            is_feature_name(f) && forwarded.contains(f),
            "{path} passes feature `{f}` that capi/Cargo.toml [features] does not forward"
        );
    }
}

#[test]
fn cmake_features_are_forwarded() {
    check("capi/yakogl_capi.cmake", cmake_features(), &capi_features());
}

#[test]
fn baremetal_features_are_forwarded() {
    check("baremetal.mk", baremetal_features(), &capi_features());
}

#[test]
fn zephyr_features_are_forwarded() {
    check("zephyr/CMakeLists.txt", zephyr_features(), &capi_features());
}

#[test]
fn opt_in_es3_features_are_forwarded() {
    let f = capi_features();
    for n in ["msaa-fbo", "color-buffer-float", "pvr-instance-divisor"] {
        assert!(f.contains(n), "capi/Cargo.toml does not forward `{n}`");
    }
}
