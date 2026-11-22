// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A shape lint over `tools/shaders/known-uncompilable-v3d71*.txt`: the V3D
//! 7.1 online back end's refusal lists -- `known-uncompilable-v3d71.txt` and
//! every sibling a 7.1 corpus test shares it with (`-glmark2`, `-es2`,
//! `-deqp-gen`, `-tree`, `-mali`, ...), globbed at run time. The 4.2 list
//! (`known-uncompilable-v3d.txt`) and every other file in `tools/shaders`
//! are not names this pattern reaches.
//!
//! Contents are not this lint's business -- which programs refuse, whether a
//! listed refusal still refuses, and what a reason says are
//! `tests/v3d71_online_corpus.rs`'s (and the other 7.1 corpus tests' and
//! stale71's). This test only checks the shape, so it passes on whatever
//! subset of these files exists when it runs, including when only the current
//! one does. It fails a file on:
//!
//! * a line the list readers cannot parse. `tests/v3d71_online_corpus.rs`'s
//!   `listed()` keeps every line that does not start `#` and is not blank,
//!   and splits each at the first `": "` into `<key>: <message>`; so every
//!   entry line needs that split with a non-empty key that holds no `':'`
//!   and a non-empty message. Leading whitespace (an indented line, or an
//!   indented `#` comment the readers would take for an entry) and trailing
//!   whitespace are malformations: a trailing space would hide inside the
//!   message the corpus tests match byte for byte. A line blank but for
//!   spaces or tabs is a malformation.
//! * an entry key listed twice (`listed()` asserts each key once, and a
//!   doubled key hides which entry is live).
//! * an entry without a reason, in the file's own convention. A file whose
//!   entries carry a trailing `# reason` (a `#` with a space before it, a
//!   space after it, and text after that space -- the front end's own
//!   messages hold `#` tokens glued to words, `'#version'` or
//!   `#if without #endif`, and those stay part of the message) must carry
//!   one on every entry. Otherwise the reason is the `#` lines above the
//!   entry, as in `known-uncompilable-v3d71.txt`: every group of entry
//!   lines (groups are separated by blank lines) needs at least one `#` line
//!   with text after the `#` since the previous group, and one such line
//!   covers its whole group. A bare `#` line and the `# SPDX-...` header
//!   lines are shape, not reason.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test shaderc71_lists
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/shaders")
}

/// Every `tools/shaders/known-uncompilable-v3d71*.txt` present at run time,
/// sorted by name.
fn lists() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(root()).expect("tools/shaders is readable") {
        let p = e.expect("a directory entry").path();
        if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            if n.starts_with("known-uncompilable-v3d71") && n.ends_with(".txt") {
                out.push(p);
            }
        }
    }
    out.sort();
    /* The population first: a lint reading no file would prove nothing. */
    assert!(
        !out.is_empty(),
        "no tools/shaders/known-uncompilable-v3d71*.txt exists: the 7.1 refusal lists this lint checks are gone"
    );
    out
}

/// The start of `l`'s trailing reason: the rightmost `#` that has a space
/// before it, a space after it, and text after that space. A `#` glued to a
/// word (the front end's `#version`, `#endif`, `'#version'`) is part of the
/// message, not a reason.
fn trailing_reason(l: &str) -> Option<usize> {
    let mut from = l.len();
    while let Some(j) = l[..from].rfind("# ") {
        if j > 0 && l.as_bytes()[j - 1] == b' ' && !l[j + 2..].trim().is_empty() {
            return Some(j);
        }
        from = j;
    }
    None
}

/// One list linted: the problems, one line each
/// (`<file>:<line>: <problem>`), empty when the file is well-formed. `rel`
/// names it in the messages.
fn lint(rel: &str, text: &str) -> Vec<String> {
    let mut bad: Vec<String> = Vec::new();
    /* (line, key, trailing reason, reason open when the entry was seen) */
    let mut entries: Vec<(usize, &str, Option<&str>, bool)> = Vec::new();
    let mut keys: BTreeMap<&str, usize> = BTreeMap::new();
    /* A group's reason: a `#` line with text (not bare `#`, not SPDX) seen
     * since the last blank line. Entry lines do not close it, so one reason
     * covers its whole group, as `known-uncompilable-v3d71.txt` writes them. */
    let mut open = false;

    for (i, l) in text.lines().enumerate() {
        let n = i + 1;
        if let Some(rest) = l.strip_prefix('#') {
            let body = rest.trim();
            if !body.is_empty() && !body.starts_with("SPDX-") {
                open = true;
            }
            continue;
        }
        if l.is_empty() {
            open = false;
            continue;
        }
        if l.trim().is_empty() {
            bad.push(format!("{rel}:{n}: blank but for spaces (delete the line)"));
            open = false;
            continue;
        }

        /* The entry line proper. Format first: what `listed()` needs to read
         * it at all, whatever convention the file names reasons by. */
        if l != l.trim_start() {
            bad.push(format!("{rel}:{n}: indented (a line is an entry at column 0 or a `#` comment)"));
        }
        let at = trailing_reason(l);
        let (body, reason) = match at {
            Some(j) => (l[..j].trim_end(), Some(l[j + 2..].trim())),
            None => (l, None),
        };
        if at.is_none() && l != l.trim_end() {
            bad.push(format!("{rel}:{n}: trailing whitespace (it would hide inside the message)"));
        }
        match body.split_once(": ") {
            None => bad.push(format!("{rel}:{n}: not `<key>: <message>`: {body}")),
            Some((k, m)) => {
                if k.is_empty() {
                    bad.push(format!("{rel}:{n}: empty key"));
                } else if k.contains(':') {
                    bad.push(format!("{rel}:{n}: the key holds a ':': {k}"));
                } else if m.is_empty() {
                    bad.push(format!("{rel}:{n}: empty message: {k}"));
                } else if k != k.trim() {
                    bad.push(format!("{rel}:{n}: whitespace around the key (a space before the ':')"));
                } else if let Some(old) = keys.insert(k, n) {
                    bad.push(format!("{rel}:{n}: {k} is listed twice (also line {old})"));
                } else {
                    entries.push((n, k, reason, open));
                }
            }
        }
    }

    /* The reason, in the file's own convention: trailing reasons if any
     * entry carries one, `#` lines above each entry group otherwise. */
    let trailing = entries.iter().any(|(_, _, reason, _)| reason.is_some());
    for (n, key, reason, group_open) in &entries {
        if trailing {
            if reason.is_none() {
                bad.push(format!("{rel}:{n}: entry without a `# reason` at the line end"));
            }
        } else if !*group_open {
            bad.push(format!("{rel}:{n}: entry without a `#` reason above its group: {key}"));
        }
    }
    bad.sort();
    bad
}

#[test]
fn every_v3d71_list_line_is_well_formed_and_has_a_reason() {
    let mut bad = Vec::new();
    for p in lists() {
        let rel = p.strip_prefix(root()).unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{rel}: {e}"));
        bad.extend(lint(&rel, &text));
    }
    assert!(bad.is_empty(), "tools/shaders/known-uncompilable-v3d71*.txt is malformed:\n{}", bad.join("\n"));
}

/// The lint flags each malformation it lists, on a fixture built here: a
/// doubled key, a whitespace-only line, an indented line, a line the readers
/// cannot parse, and an entry with no reason above its group. And a `#` glued
/// to a word inside a message is no trailing reason: the convention is read
/// from the file, not from a stray `#` -- the fixture below keeps the leading
/// convention even though its last entry's message names `#version`.
#[test]
fn the_lint_flags_each_malformation_it_lists() {
    let block = [
        "# SPDX-License-Identifier: Apache-2.0",
        "# reason: x.vert is desktop GLSL and the front end is an OpenGL ES compiler",
        "x.vert: front end x.vert: desktop GLSL source; this is an OpenGL ES compiler",
        "x.vert: front end x.vert: another message",
        "   ",
        " no_colon_here",
        "y.vert:",
        "",
        "y.vert: front end y.vert: #version requires a version number",
    ]
    .join("\n");
    let bad = lint("block.txt", &block);
    for want in [
        "block.txt:4: x.vert is listed twice",
        "block.txt:5: blank but for spaces",
        "block.txt:6: indented",
        "block.txt:7: not `<key>: <message>`",
        "block.txt:9: entry without a `#` reason above its group: y.vert",
    ] {
        assert!(bad.iter().any(|b| b.contains(want)), "{want}\n{}", bad.join("\n"));
    }
    assert_eq!(bad.len(), 6, "the fixture's six problems (line 6 twice)\n{}", bad.join("\n"));
    assert!(
        !bad.iter().any(|b| b.contains("at the line end")),
        "a message's `#version` was read as a trailing reason:\n{}",
        bad.join("\n")
    );

    /* A file naming its reasons at the line end: every entry needs one. */
    let tail = [
        "# SPDX-License-Identifier: Apache-2.0",
        "a + b: Select(Unsupported) # Mesa 7.1 refuses this pair too",
        "a + c: Alloc(DoesNotFit)",
    ]
    .join("\n");
    let bad = lint("tail.txt", &tail);
    assert_eq!(bad.join("\n"), "tail.txt:3: entry without a `# reason` at the line end");

    /* Well-formed in the trailing convention: reasons at the line end, no
     * `#` block above the entries. */
    let good = [
        "# SPDX-License-Identifier: Apache-2.0",
        "a + b: Alloc(DoesNotFit) # Mesa 7.1 spills it; we do not",
        "",
        "a + c: Select(Unsupported) # the 7.1 selector refuses this shape",
    ]
    .join("\n");
    assert_eq!(lint("good.txt", &good).join("\n"), "");
}
