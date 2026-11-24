// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every cargo feature a C build path passes is forwarded by capi/Cargo.toml.
//!
//! Every C build of YakoGL ends in `cargo build -p yakogl-capi
//! --no-default-features --features <list>`. A name in that list that is not
//! a key of capi/Cargo.toml's `[features]` stops the build ("the package
//! 'yakogl-capi' does not contain this feature"), and the build that passes
//! it is usually a board image nobody builds on a pull request: the opt-in
//! ES 3.0 features `msaa-fbo`, `color-buffer-float` and `pvr-instance-divisor`
//! each had a CMake option before capi forwarded them.
//!
//! This test reads the files and builds nothing. The names come from:
//! - capi/yakogl_capi.cmake: every `list(APPEND _features "<name>")` of
//!   yakogl_add_capi_archive(), `osal-${A_OSAL}` and `gpu-${A_GPU}` expanded
//!   with the values the callers and the function's own check allow;
//! - CMakeLists.txt and zephyr/CMakeLists.txt: every "the crate's `<name>`
//!   feature" their comments and option texts name, and the Zephyr file's
//!   `set(YAKOGL_<X> ON)` switches (the Kconfig-to-feature mapping), each of
//!   which must be a switch yakogl_capi.cmake turns into a feature;
//! - baremetal.mk: every `CARGO_FEATURES :=` line;
//! - tools/hostharness/glmark2/mali/CMakeLists.txt: its literal `FEATURES`.
//!
//! Not covered: `YAKOGL_EXTRA_FEATURES` (tools/cts/host-deqp-run.sh's
//! EXTRA_FEATURES), whose names are the caller's and exist only at configure
//! time.
//!
//! Run: `cargo test --test capi_feature_forward --features gpu-vivante,shaderc-online`
//! (the test itself needs no feature, but the crate needs one GPU feature and
//! one shader back end; without them the library does not compile. Any
//! feature set of tools/check.sh's test rows does.)

use std::collections::{BTreeMap, BTreeSet};

fn read(path: &str) -> String {
    let full = format!("{}/{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"))
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The `[features]` table of a manifest: key -> the value's text.
fn features_table(manifest: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut inside = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[features]";
        } else if inside && !line.starts_with('#') {
            if let Some((k, v)) = line.split_once('=') {
                out.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    out
}

/// Every `<name>` of "the crate's `<name>` feature" in a CMake file; the
/// phrase may wrap onto the next comment line and the backticks are optional.
fn crate_feature_mentions(text: &str) -> BTreeSet<String> {
    const MARK: &str = "the crate's";
    let filler = |c: char| c.is_whitespace() || c == '#' || c == '`';
    let mut out = BTreeSet::new();
    for (at, _) in text.match_indices(MARK) {
        let rest = text[at + MARK.len()..].trim_start_matches(filler);
        let len =
            rest.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')).unwrap_or(rest.len());
        if len > 0 && rest[len..].trim_start_matches(filler).starts_with("feature") {
            out.insert(rest[..len].to_string());
        }
    }
    out
}

/// What yakogl_add_capi_archive() appends to its feature list.
struct CapiCmake {
    /// The literals of `list(APPEND _features "<literal>")`, unexpanded.
    appended: BTreeSet<String>,
    /// `if(<VARIABLE>)` -> the feature appended directly under it.
    switches: BTreeMap<String, String>,
}

fn scan_capi_cmake(text: &str) -> CapiCmake {
    const APPEND: &str = "list(APPEND _features \"";
    let mut scan = CapiCmake { appended: BTreeSet::new(), switches: BTreeMap::new() };
    // The conditions of the open if() blocks, innermost last ("" once an
    // else/elseif has been passed: the branch is no longer the bare switch).
    let mut open: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(cond) = line.strip_prefix("if(") {
            open.push(cond.trim_end_matches(')').to_string());
        } else if line.starts_with("elseif(") || line.starts_with("else(") {
            if let Some(top) = open.last_mut() {
                top.clear();
            }
        } else if line.starts_with("endif(") {
            open.pop();
        } else if let Some(rest) = line.strip_prefix(APPEND) {
            let literal = rest.split('"').next().unwrap_or("").to_string();
            if let Some(var) = open.last().filter(|c| {
                !c.is_empty() && c.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            }) {
                scan.switches.insert(var.clone(), literal.clone());
            }
            scan.appended.insert(literal);
        }
    }
    scan
}

/// The words after `marker` up to the closing parenthesis, on each line that
/// starts with it (`set_property(CACHE YAKOGL_OSAL PROPERTY STRINGS a b c)`).
fn words_after(text: &str, marker: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix(marker) {
            let rest = rest.split(')').next().unwrap_or("");
            out.extend(rest.split_whitespace().map(|w| w.trim_matches('"').to_string()).filter(|w| is_name(w)));
        }
    }
    out
}

/// The feature names each build path passes: (where it comes from, name).
fn passed() -> Vec<(&'static str, String)> {
    let capi_cmake = read("capi/yakogl_capi.cmake");
    let root = read("CMakeLists.txt");
    let zephyr = read("zephyr/CMakeLists.txt");
    let mut out: Vec<(&'static str, String)> = Vec::new();

    // The values of yakogl_add_capi_archive()'s OSAL and GPU arguments: the
    // root build's cache choices, the Zephyr build's literals, and the list
    // the function itself accepts.
    let mut osals = words_after(&root, "set_property(CACHE YAKOGL_OSAL PROPERTY STRINGS");
    osals.extend(words_after(&zephyr, "OSAL "));
    let mut gpus = words_after(&root, "set_property(CACHE YAKOGL_GPU PROPERTY STRINGS");
    gpus.extend(words_after(&zephyr, "set(YAKOGL_GPU "));
    for line in capi_cmake.lines().filter(|l| l.contains("A_GPU MATCHES \"^(")) {
        let list = line.split("\"^(").nth(1).and_then(|r| r.split(")$\"").next()).unwrap_or("");
        gpus.extend(list.split('|').map(str::to_string));
    }
    assert!(osals.len() >= 3 && osals.contains("zephyr"), "the OSAL values were not found: {osals:?}");
    assert!(gpus.len() >= 4 && gpus.contains("powervr"), "the GPU values were not found: {gpus:?}");

    for literal in scan_capi_cmake(&capi_cmake).appended {
        if let Some(prefix) = literal.strip_suffix("${A_OSAL}") {
            out.extend(osals.iter().map(|v| ("capi/yakogl_capi.cmake (OSAL)", format!("{prefix}{v}"))));
        } else if let Some(prefix) = literal.strip_suffix("${A_GPU}") {
            out.extend(gpus.iter().map(|v| ("capi/yakogl_capi.cmake (GPU)", format!("{prefix}{v}"))));
        } else {
            // A new `${...}` shape lands here and fails as "not a feature
            // name": teach this test its values.
            out.push(("capi/yakogl_capi.cmake", literal));
        }
    }
    out.extend(crate_feature_mentions(&root).into_iter().map(|n| ("CMakeLists.txt", n)));
    out.extend(crate_feature_mentions(&zephyr).into_iter().map(|n| ("zephyr/CMakeLists.txt", n)));
    for line in read("baremetal.mk").lines() {
        if let Some(list) = line.trim().strip_prefix("CARGO_FEATURES :=") {
            // `$(CARGO_FEATURES)` is the list so far.
            out.extend(
                list.split(',')
                    .map(str::trim)
                    .filter(|n| *n != "$(CARGO_FEATURES)")
                    .map(|n| ("baremetal.mk", n.to_string())),
            );
        }
    }
    let mali = read("tools/hostharness/glmark2/mali/CMakeLists.txt");
    for line in mali.lines().map(str::trim) {
        if let Some(list) = line.strip_prefix("FEATURES ") {
            out.extend(
                list.split_whitespace().map(|n| ("tools/hostharness/glmark2/mali/CMakeLists.txt", n.to_string())),
            );
        }
    }
    out
}

#[test]
fn every_feature_a_build_path_passes_is_forwarded() {
    let capi = features_table(&read("capi/Cargo.toml"));
    let passed = passed();
    // Each source was really read: a renamed variable must not turn the test
    // into one that checks nothing.
    for (source, at_least) in [
        ("capi/yakogl_capi.cmake", 8),
        ("capi/yakogl_capi.cmake (OSAL)", 3),
        ("capi/yakogl_capi.cmake (GPU)", 4),
        ("CMakeLists.txt", 5),
        ("zephyr/CMakeLists.txt", 4),
        ("baremetal.mk", 5),
        ("tools/hostharness/glmark2/mali/CMakeLists.txt", 5),
    ] {
        let n = passed.iter().filter(|(s, _)| *s == source).count();
        assert!(
            n >= at_least,
            "{source}: {n} feature name(s) found, at least {at_least} expected; has its shape changed?"
        );
    }
    let mut missing = Vec::new();
    for (source, name) in &passed {
        if !is_name(name) {
            missing.push(format!("{source}: `{name}` is not a feature name this test can resolve"));
            continue;
        }
        match capi.get(name) {
            None => missing.push(format!("{source} passes `{name}`, which capi/Cargo.toml's [features] does not have")),
            // gl-test-runner is capi's own (an optional dependency); every
            // other one is the yakogl crate's feature of the same name.
            Some(v) if v.contains("\"dep:") => {}
            Some(v) if *v == format!("[\"yakogl/{name}\"]") => {}
            Some(v) => missing.push(format!("capi/Cargo.toml: `{name} = {v}` does not forward to yakogl/{name}")),
        }
    }
    assert!(
        missing.is_empty(),
        "add `name = [\"yakogl/name\"]` to capi/Cargo.toml's [features]:\n{}",
        missing.join("\n")
    );
}

/// The Kconfig-to-feature mapping: zephyr/CMakeLists.txt turns a Kconfig
/// symbol into `set(YAKOGL_<X> ON)`, and that only reaches cargo when
/// yakogl_add_capi_archive() has an `if(YAKOGL_<X>)` (or, for the one passed
/// as an argument, `if(A_<X>)`) that appends a forwarded feature.
#[test]
fn every_zephyr_switch_reaches_a_forwarded_feature() {
    let capi = features_table(&read("capi/Cargo.toml"));
    let switches = scan_capi_cmake(&read("capi/yakogl_capi.cmake")).switches;
    let zephyr = read("zephyr/CMakeLists.txt");
    let mut seen = BTreeSet::new();
    for line in zephyr.lines().map(str::trim) {
        let Some(var) = line.strip_prefix("set(YAKOGL_").and_then(|r| r.strip_suffix(" ON)")) else { continue };
        seen.insert(var.to_string());
        let feature = switches
            .get(&format!("YAKOGL_{var}"))
            .or_else(|| switches.get(&format!("A_{var}")))
            .unwrap_or_else(|| panic!("zephyr/CMakeLists.txt sets YAKOGL_{var}, which yakogl_add_capi_archive() does not turn into a feature"));
        assert!(
            capi.contains_key(feature),
            "YAKOGL_{var} passes `{feature}`, which capi/Cargo.toml's [features] does not have"
        );
    }
    for var in ["NO_SHADER_TABLE", "REPORT_ES2", "TEST_KNOBS", "GL_TEST_RUNNER", "PVR_INSTANCE_DIVISOR"] {
        assert!(seen.contains(var), "zephyr/CMakeLists.txt no longer sets YAKOGL_{var}: {seen:?}");
    }
}

/// The opt-in ES 3.0 features by name: each has a CMake option and none is
/// in a default build, so nothing else notices one that is not forwarded.
#[test]
fn the_opt_in_es3_features_are_forwarded() {
    let capi = features_table(&read("capi/Cargo.toml"));
    let root = features_table(&read("Cargo.toml"));
    for name in ["msaa-fbo", "color-buffer-float", "pvr-instance-divisor"] {
        assert!(root.contains_key(name), "the yakogl crate has no feature `{name}`");
        assert_eq!(
            capi.get(name).map(String::as_str),
            Some(format!("[\"yakogl/{name}\"]").as_str()),
            "capi/Cargo.toml: {name}"
        );
    }
}

/// The scanners on a fixture: an unforwarded name is found, in each shape.
#[test]
fn the_scanners_find_the_names() {
    let cmake = "
        if(A_OSAL)
          list(APPEND _features \"osal-${A_OSAL}\")
        endif()
        if(A_SHADERC STREQUAL \"online\")
          list(APPEND _features \"shaderc-online\")
        else()
          list(APPEND _features \"mesa-reference-table\")
        endif()
        if(A_NO_SHADER_TABLE)
          if(NOT A_SHADERC STREQUAL \"online\")
            message(FATAL_ERROR \"no\")
          endif()
          list(APPEND _features \"no-shader-table\")
        endif()
        # The new one (the crate's
        # `brand-new` feature), a global too.
        if(YAKOGL_BRAND_NEW)
          list(APPEND _features \"brand-new\")
        endif()
        option(YAKOGL_OTHER \"another (the crate's other-one feature)\" OFF)
        # (the crate's `gpu-<name>`.
    ";
    let scan = scan_capi_cmake(cmake);
    let appended: Vec<&str> = scan.appended.iter().map(String::as_str).collect();
    assert_eq!(appended, ["brand-new", "mesa-reference-table", "no-shader-table", "osal-${A_OSAL}", "shaderc-online"]);
    let switches: Vec<(&str, &str)> = scan.switches.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        switches,
        [("A_NO_SHADER_TABLE", "no-shader-table"), ("A_OSAL", "osal-${A_OSAL}"), ("YAKOGL_BRAND_NEW", "brand-new")]
    );
    let mentions: Vec<String> = crate_feature_mentions(cmake).into_iter().collect();
    assert_eq!(mentions, ["brand-new", "other-one"]);
    let table =
        features_table("[package]\nname = \"x\"\n[features]\n# c\na-b = [\"yakogl/a-b\"]\n[dependencies]\ny = \"1\"\n");
    assert_eq!(table.into_iter().collect::<Vec<_>>(), [("a-b".to_string(), "[\"yakogl/a-b\"]".to_string())]);
    let words = words_after(
        "  set_property(CACHE X PROPERTY STRINGS zephyr linux)\n# set_property(CACHE X PROPERTY STRINGS no)\n",
        "set_property(CACHE X PROPERTY STRINGS",
    );
    assert_eq!(words.into_iter().collect::<Vec<_>>(), ["linux", "zephyr"]);
}
