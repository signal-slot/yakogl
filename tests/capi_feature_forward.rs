// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every cargo feature a C build path passes to `-p yakogl-capi` is a key of
//! capi/Cargo.toml's `[features]`.
//!
//! Each C build of yakogl ends in `cargo build -p yakogl-capi
//! --no-default-features --features <list>`. A name in that list that the
//! capi crate does not declare (as `name = ["yakogl/name"]`) is refused by
//! cargo, so the CMake option or the Kconfig switch that asks for it breaks the
//! build (`pvr-instance-divisor`, `msaa-fbo` and `color-buffer-float` were
//! all in that state until they were forwarded). This test reads the build
//! files as text and checks the names they can pass:
//!
//! - capi/yakogl_capi.cmake, `yakogl_add_capi_archive()`: every
//!   `list(APPEND _features "...")`, with `osal-${A_OSAL}` expanded over the
//!   OSALs the root CMakeLists.txt and zephyr/CMakeLists.txt pass and
//!   `gpu-${A_GPU}` over the GPUs the function accepts. The root
//!   CMakeLists.txt and zephyr/CMakeLists.txt reach those names through the
//!   `YAKOGL_*` variables the function reads; the test checks that each
//!   variable they set (CMake `option(...)`, the Kconfig `set(... ON)`) is one
//!   the function maps to a feature, and that each GPU zephyr picks is one of
//!   the function's.
//! - baremetal.mk: every name of its `CARGO_FEATURES` assignments.
//!
//! `YAKOGL_EXTRA_FEATURES` (tools/cts/host-deqp-run.sh) is free text and is
//! not checked. The five files are compiled in (`include_str!`): the test
//! opens nothing at run time, a change of any of them rebuilds it, and so the
//! gate's test cache runs it again (no TEST_TREE_PATHS entry needed). It
//! calls nothing of the library, so any feature set the library builds with
//! runs it (a bare `cargo test` does not build the library: it needs a GPU
//! and a shader back end):
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test capi_feature_forward
//! ```

use std::collections::{BTreeMap, BTreeSet};

const CAPI_CARGO_TOML: &str = include_str!("../capi/Cargo.toml");
const ROOT_CMAKE: &str = include_str!("../CMakeLists.txt");
const ZEPHYR_CMAKE: &str = include_str!("../zephyr/CMakeLists.txt");
const CAPI_CMAKE: &str = include_str!("../capi/yakogl_capi.cmake");
const BAREMETAL_MK: &str = include_str!("../baremetal.mk");

/// The keys of the `[features]` table of a Cargo.toml.
fn cargo_features(toml: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut inside = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[features]";
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            keys.insert(key.trim().trim_matches('"').to_string());
        }
    }
    keys
}

/// The text between `function(<name>)` and its `endfunction()`.
fn cmake_function<'a>(src: &'a str, name: &str) -> &'a str {
    let open = format!("function({name})");
    let at = src.find(&open).unwrap_or_else(|| panic!("no {open} in the CMake source"));
    let body = &src[at + open.len()..];
    let end = body.find("endfunction(").unwrap_or_else(|| panic!("{open} has no endfunction()"));
    &body[..end]
}

/// The first `"..."` string on a line, if any.
fn quoted(line: &str) -> Option<&str> {
    let start = line.find('"')? + 1;
    let len = line[start..].find('"')?;
    Some(&line[start..start + len])
}

/// The words of a parenthesised group starting right after `prefix`.
fn words_after<'a>(src: &'a str, prefix: &str) -> Vec<&'a str> {
    let at = src.find(prefix).unwrap_or_else(|| panic!("no `{prefix}` in the CMake source"));
    let rest = &src[at + prefix.len()..];
    let end = rest.find(')').expect("unclosed group");
    rest[..end].split_whitespace().collect()
}

/// What `yakogl_add_capi_archive()` passes: the literal feature names (the
/// `${A_OSAL}` / `${A_GPU}` forms expanded over `osals` / `gpus`), and for
/// each `if(YAKOGL_X)` that appends one name, `YAKOGL_X -> name`.
struct CapiCmake {
    names: BTreeSet<String>,
    switches: BTreeMap<String, String>,
    gpus: Vec<String>,
}

fn scan_capi_cmake(body: &str, osals: &[String]) -> CapiCmake {
    let gpu_rule =
        body.lines().find(|l| l.contains("A_GPU MATCHES")).expect("yakogl_add_capi_archive() no longer checks A_GPU");
    let gpus: Vec<String> = quoted(gpu_rule)
        .expect("A_GPU MATCHES without a quoted pattern")
        .trim_start_matches("^(")
        .trim_end_matches(")$")
        .split('|')
        .map(str::to_string)
        .collect();
    let mut names = BTreeSet::new();
    let mut switches = BTreeMap::new();
    let mut pending_if: Option<String> = None;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(var) = line
            .strip_prefix("if(")
            .and_then(|r| r.strip_suffix(')'))
            .filter(|v| v.starts_with("YAKOGL_") && !v.contains(' '))
        {
            pending_if = Some(var.to_string());
            continue;
        }
        if line.starts_with("endif(") || line.starts_with("else") {
            pending_if = None;
            continue;
        }
        if !line.starts_with("list(APPEND _features") {
            continue;
        }
        // `list(APPEND _features ${_extra})`: YAKOGL_EXTRA_FEATURES, free text.
        let Some(name) = quoted(line) else { continue };
        let expanded: Vec<String> = if let Some(prefix) = name.strip_suffix("${A_OSAL}") {
            osals.iter().map(|o| format!("{prefix}{o}")).collect()
        } else if let Some(prefix) = name.strip_suffix("${A_GPU}") {
            gpus.iter().map(|g| format!("{prefix}{g}")).collect()
        } else {
            assert!(!name.contains("${"), "yakogl_add_capi_archive() appends `{name}`: teach this test the expansion");
            vec![name.to_string()]
        };
        if let (Some(var), [one]) = (pending_if.take(), expanded.as_slice()) {
            switches.insert(var, one.clone());
        }
        names.extend(expanded);
    }
    CapiCmake { names, switches, gpus }
}

/// The OSAL names a `yakogl_add_capi_archive()` caller passes: the root's
/// `YAKOGL_OSAL` cache strings and every literal `OSAL <name>` argument.
fn osals(root: &str, zephyr: &str) -> Vec<String> {
    let mut out: BTreeSet<String> =
        words_after(root, "set_property(CACHE YAKOGL_OSAL PROPERTY STRINGS").into_iter().map(str::to_string).collect();
    for src in [root, zephyr] {
        for line in src.lines() {
            if let Some(name) = line.trim().strip_prefix("OSAL ") {
                let name = name.trim();
                if !name.starts_with('"') {
                    out.insert(name.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// The `YAKOGL_*` variables a file turns on: `option(YAKOGL_X ...)` (CMake)
/// and `set(YAKOGL_X ON)` (the zephyr Kconfig mapping).
fn switches_set(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("option(") {
            if let Some(var) = rest.split_whitespace().next() {
                out.insert(var.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("set(") {
            let mut w = rest.trim_end_matches(')').split_whitespace();
            if let (Some(var), Some("ON")) = (w.next(), w.next()) {
                out.insert(var.to_string());
            }
        }
    }
    out.retain(|v| v.starts_with("YAKOGL_"));
    out
}

/// The GPUs zephyr/CMakeLists.txt picks: `set(YAKOGL_GPU "<name>")`.
fn zephyr_gpus(zephyr: &str) -> Vec<String> {
    zephyr
        .lines()
        .filter(|l| l.trim().starts_with("set(YAKOGL_GPU "))
        .filter_map(|l| quoted(l).map(str::to_string))
        .collect()
}

/// Every name of baremetal.mk's `CARGO_FEATURES` assignments.
fn baremetal_features(mk: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in mk.lines() {
        let Some(rest) = line.trim().strip_prefix("CARGO_FEATURES") else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };
        for name in value.split(',') {
            let name = name.trim();
            if !name.is_empty() && !name.starts_with("$(") {
                out.insert(name.to_string());
            }
        }
    }
    out
}

fn unforwarded<'a>(passed: &'a BTreeSet<String>, capi: &BTreeSet<String>) -> Vec<&'a str> {
    passed.iter().filter(|n| !capi.contains(*n)).map(String::as_str).collect()
}

#[test]
fn every_feature_a_build_path_passes_is_forwarded() {
    let capi = cargo_features(CAPI_CARGO_TOML);
    let (root, zephyr) = (ROOT_CMAKE, ZEPHYR_CMAKE);

    let osal_names = osals(root, zephyr);
    let scan = scan_capi_cmake(cmake_function(CAPI_CMAKE, "yakogl_add_capi_archive"), &osal_names);
    let bare = baremetal_features(BAREMETAL_MK);

    // The scans must see something, or a passing test says nothing.
    for known in ["osal-linux", "osal-zephyr", "gpu-powervr", "report-es2"] {
        assert!(scan.names.contains(known), "CMake scan lost `{known}`: {:?}", scan.names);
    }
    for known in ["osal-baremetal", "gpu-powervr", "report-es2"] {
        assert!(bare.contains(known), "baremetal.mk scan lost `{known}`: {bare:?}");
    }
    for opt in ["msaa-fbo", "color-buffer-float", "pvr-instance-divisor"] {
        assert!(
            scan.switches.values().any(|f| f == opt),
            "yakogl_add_capi_archive() no longer has a switch for `{opt}`: {:?}",
            scan.switches
        );
    }

    let mut failures = Vec::new();
    let bad = unforwarded(&scan.names, &capi);
    if !bad.is_empty() {
        failures.push(format!("capi/yakogl_capi.cmake passes {bad:?}"));
    }
    let bad = unforwarded(&bare, &capi);
    if !bad.is_empty() {
        failures.push(format!("baremetal.mk passes {bad:?}"));
    }
    // Zephyr: the GPUs it picks and the Kconfig switches it turns on reach
    // yakogl_add_capi_archive() as gpu-<name> and its YAKOGL_* variables.
    for gpu in zephyr_gpus(zephyr) {
        if !scan.gpus.contains(&gpu) {
            failures.push(format!("zephyr/CMakeLists.txt picks GPU `{gpu}`, which yakogl_add_capi_archive() refuses"));
        } else if !capi.contains(&format!("gpu-{gpu}")) {
            failures.push(format!("zephyr/CMakeLists.txt picks GPU `{gpu}`: gpu-{gpu} is not forwarded"));
        }
    }
    for (file, src) in [("CMakeLists.txt", root), ("zephyr/CMakeLists.txt (the Kconfig mapping)", zephyr)] {
        for var in switches_set(src) {
            if let Some(feature) = scan.switches.get(&var) {
                if !capi.contains(feature) {
                    failures.push(format!("{file} turns on {var}: feature `{feature}` is not forwarded"));
                }
            }
        }
    }
    // The zephyr mapping sets each feature switch the function reads.
    let zephyr_sets: BTreeSet<String> = zephyr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("set("))
        .filter_map(|r| r.split_whitespace().next().map(str::to_string))
        .collect();
    for var in ["YAKOGL_REPORT_ES2", "YAKOGL_TEST_KNOBS", "YAKOGL_PVR_INSTANCE_DIVISOR"] {
        assert!(zephyr_sets.contains(var), "zephyr/CMakeLists.txt no longer sets {var}: the scan is stale");
    }
    assert!(
        failures.is_empty(),
        "features passed to -p yakogl-capi but not forwarded by capi/Cargo.toml:\n  {}",
        failures.join("\n  ")
    );
}

/// The scans on a sample with one unforwarded name find it and only it.
#[test]
fn the_scan_finds_an_unforwarded_name() {
    let capi = cargo_features(
        "[package]\nname = \"x\"\n[features]\n# c\nosal-linux = [\"yakogl/osal-linux\"]\n\
         gpu-powervr = [\"yakogl/gpu-powervr\"]\nmsaa-fbo = [\"yakogl/msaa-fbo\"]\n\
         [dependencies]\nnot-a-feature = \"1\"\n",
    );
    assert_eq!(capi.len(), 3, "{capi:?}");
    let body = r#"
  if(NOT A_GPU MATCHES "^(powervr|mali)$")
  endif()
  list(APPEND _features "osal-${A_OSAL}")
  list(APPEND _features "gpu-${A_GPU}")
  if(YAKOGL_MSAA_FBO)
    list(APPEND _features "msaa-fbo")
  endif()
  if(YAKOGL_COLOR_BUFFER_FLOAT)
    list(APPEND _features "color-buffer-float")
  endif()
  list(APPEND _features ${_extra})
"#;
    let scan = scan_capi_cmake(body, &["linux".to_string()]);
    assert_eq!(unforwarded(&scan.names, &capi), vec!["color-buffer-float", "gpu-mali"]);
    assert_eq!(scan.switches.get("YAKOGL_COLOR_BUFFER_FLOAT").map(String::as_str), Some("color-buffer-float"));
    let bare = baremetal_features(
        "CARGO_FEATURES := osal-linux,gpu-powervr\nCARGO_FEATURES := $(CARGO_FEATURES),report-es2\n",
    );
    assert_eq!(unforwarded(&bare, &capi), vec!["report-es2"]);
}
