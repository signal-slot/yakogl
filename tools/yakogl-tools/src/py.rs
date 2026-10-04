// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The pieces of Python semantics the generators relied on, reproduced where
//! Rust's standard library or the `regex` crate behave differently.
//!
//! The tools were Python scripts until they were ported here; the port is
//! meant to be behaviour-identical (same tables, same messages), so every
//! place where `str.strip`, `re`, `int()`, `fnmatch`, `os.path` or text-mode
//! file reading differ from the obvious Rust spelling goes through this module.

use crate::{Error, R};
use regex::Regex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

/* ---- regular expressions --------------------------------------------------- */

thread_local! {
    static RE_CACHE: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

/// Python's `\s` (str patterns) is `str.isspace()`: Unicode White_Space plus
/// the ASCII information separators U+001C..U+001F, which the regex crate's
/// `\s` does not match. Every `\s` in a pattern is widened accordingly (a
/// nested class is a union in the regex crate, so this also works inside
/// `[...]`).
fn widen_space(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len() + 16);
    let mut it = pat.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('s') => out.push_str(r"[\s\x1c-\x1f]"),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Compile (and cache) a Python-flavoured pattern. The patterns are written
/// as in the Python original except that `{`/`}` are escaped and the flags
/// are inline (`(?m)`, `(?s)`). `\w`, `\d` and `\b` are Unicode in both
/// engines and agree on ASCII text; `\s` is widened (see `widen_space`).
/// Callers that pass text which may contain non-ASCII code outside comments
/// use [`PW`] for `\w`.
pub fn re(pat: &str) -> Regex {
    RE_CACHE.with(|c| {
        if let Some(r) = c.borrow().get(pat) {
            return r.clone();
        }
        let r = Regex::new(&widen_space(pat)).unwrap_or_else(|e| panic!("bad pattern {pat}: {e}"));
        c.borrow_mut().insert(pat.to_string(), r.clone());
        r
    })
}

/// Python's `\w` for str patterns (`str.isalnum()` or `_`): letters and all
/// numeric categories. The regex crate's `\w` also takes combining marks and
/// excludes No/Nl, which only matters for non-ASCII text.
pub const PW: &str = r"[_\p{L}\p{N}]";

/// `re.escape` equivalent (the escaped text matches itself literally in both).
pub fn esc(s: &str) -> String {
    regex::escape(s)
}

/// Python `re.sub(pattern, repl, text)` with a literal replacement.
pub fn sub_lit(r: &Regex, text: &str, repl: &str) -> String {
    r.replace_all(text, regex::NoExpand(repl)).into_owned()
}

/// Python `re.sub` with a callback that may raise: the first error stops the
/// substitution and is returned.
pub fn sub_fn(r: &Regex, text: &str, mut f: impl FnMut(&regex::Captures) -> R<String>) -> R<String> {
    let mut err = None;
    let out = r.replace_all(text, |c: &regex::Captures| {
        if err.is_some() {
            return String::new();
        }
        match f(c) {
            Ok(s) => s,
            Err(e) => {
                err = Some(e);
                String::new()
            }
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(out.into_owned()),
    }
}

/// A capture group as Python's `m.group(n)` (None when it did not take part).
pub fn grp<'t>(c: &regex::Captures<'t>, n: usize) -> Option<&'t str> {
    c.get(n).map(|m| m.as_str())
}

/// A capture group that always takes part in a match of its pattern.
pub fn g<'t>(c: &regex::Captures<'t>, n: usize) -> &'t str {
    c.get(n).map(|m| m.as_str()).unwrap_or("")
}

/* ---- strings ----------------------------------------------------------------- */

/// `str.isspace()` for one character.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// `str.rstrip()`.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_py_space)
}

/// `str.partition(sep)`.
pub fn partition<'a>(s: &'a str, sep: &str) -> (&'a str, &'a str) {
    match s.find(sep) {
        Some(i) => (&s[..i], &s[i + sep.len()..]),
        None => (s, ""),
    }
}

/// `"%-Ns" % s` (padding counts code points, as Rust's `{:<N}` does).
pub fn ljust(s: &str, n: usize) -> String {
    format!("{s:<n$}")
}

/// `repr(s)` of a str (error messages only).
pub fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c)
            }
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if c.is_control() => {
                let v = c as u32;
                if v <= 0xff {
                    out.push_str(&format!("\\x{v:02x}"))
                } else if v <= 0xffff {
                    out.push_str(&format!("\\u{v:04x}"))
                } else {
                    out.push_str(&format!("\\U{v:08x}"))
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `"%x"`/`"%#x"` of a possibly negative integer (Python prints a sign).
pub fn hex(v: i128, alt: bool) -> String {
    let p = if alt { "0x" } else { "" };
    if v < 0 {
        format!("-{p}{:x}", v.unsigned_abs())
    } else {
        format!("{p}{v:x}")
    }
}

/* ---- integers ---------------------------------------------------------------- */

/// The decimal value of a Unicode Nd character: Nd characters come in runs of
/// ten consecutive code points starting at a zero, so the value is the
/// number of Nd characters immediately below it, modulo 10.
fn nd_value(c: char) -> Option<u32> {
    if let Some(d) = c.to_digit(10) {
        return Some(d);
    }
    let nd = re(r"^\p{Nd}$");
    let mut buf = [0u8; 4];
    if !nd.is_match(c.encode_utf8(&mut buf)) {
        return None;
    }
    let mut k = 0u32;
    let mut v = c as u32;
    while v > 0 {
        v -= 1;
        match char::from_u32(v) {
            Some(p) if nd.is_match(p.encode_utf8(&mut buf)) => k += 1,
            _ => break,
        }
    }
    Some(k % 10)
}

fn too_big(s: &str) -> Error {
    Error::Fatal(format!("integer {} is out of the supported range", repr(s)))
}

/// `int(s)` of a string a `\d+` group matched (Unicode decimal digits).
pub fn pint(s: &str) -> R<i64> {
    let mut v: i64 = 0;
    for c in s.chars() {
        let d = nd_value(c).ok_or_else(|| Error::Fatal(format!("invalid literal for int(): {}", repr(s))))?;
        v = v.checked_mul(10).and_then(|v| v.checked_add(d as i64)).ok_or_else(|| too_big(s))?;
    }
    Ok(v)
}

/// `int(s, base)` for base 10 (`base0 == false`) or base 0 (prefix-detected).
pub fn py_int_str(s: &str, base0: bool) -> R<i128> {
    let bad =
        || Error::Fatal(format!("invalid literal for int() with base {}: {}", if base0 { 0 } else { 10 }, repr(s)));
    let t = strip(s);
    let (neg, t) = match t.chars().next() {
        Some('-') => (true, &t[1..]),
        Some('+') => (false, &t[1..]),
        _ => (false, t),
    };
    let lower = t.to_ascii_lowercase();
    let (radix, digits) = if base0 && (lower.starts_with("0x") || lower.starts_with("0o") || lower.starts_with("0b")) {
        let r = match &lower[1..2] {
            "x" => 16,
            "o" => 8,
            _ => 2,
        };
        // int("0x_1f", 0) is valid: an underscore may follow the prefix.
        let rest = &t[2..];
        (r, rest.strip_prefix('_').unwrap_or(rest))
    } else {
        (10, t)
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return Err(bad());
    }
    let mut v: i128 = 0;
    let mut all_zero = true;
    for c in digits.chars() {
        if c == '_' {
            continue;
        }
        let d = if radix == 10 { nd_value(c) } else { c.to_digit(radix) }.ok_or_else(bad)?;
        if d != 0 {
            all_zero = false;
        }
        v = v.checked_mul(radix as i128).and_then(|v| v.checked_add(d as i128)).ok_or_else(|| too_big(s))?;
    }
    // Base 0: a decimal literal may not have leading zeros ("0" and "00" are fine).
    if base0 && radix == 10 && digits.len() > 1 && digits.starts_with('0') && !all_zero {
        return Err(bad());
    }
    Ok(if neg { -v } else { v })
}

/// `int(x)` of a JSON value (json.load gives int/float/str/bool/None/...).
pub fn py_int_json(v: &serde_json::Value, base0: bool) -> R<i128> {
    use serde_json::Value;
    match v {
        Value::Bool(b) if !base0 => Ok(*b as i128),
        Value::Number(n) if !base0 => {
            if let Some(i) = n.as_i64() {
                Ok(i as i128)
            } else if let Some(u) = n.as_u64() {
                Ok(u as i128)
            } else {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if !f.is_finite() || f.abs() >= 1e38 {
                    return Err(Error::Fatal(format!("cannot convert float {f} to integer")));
                }
                Ok(f.trunc() as i128)
            }
        }
        Value::String(s) => py_int_str(s, base0),
        other => Err(Error::Fatal(format!("int() argument must be a string or a number, not {other}"))),
    }
}

/// `str(x)` of a JSON value for the scalar cases a sidecar note can hold.
pub fn py_str_json(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        other => other.to_string(),
    }
}

/// Python truthiness of a JSON value.
pub fn truthy_json(v: &serde_json::Value) -> bool {
    use serde_json::Value;
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/* ---- files and paths --------------------------------------------------------- */

/// The value of an environment variable when Python would find it truthy
/// (`os.environ.get(X)` set and non-empty).
pub fn env_set(name: &str) -> Option<String> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn io_fatal(what: &str, path: &str, e: std::io::Error) -> Error {
    Error::Fatal(format!("{what} {path}: {e}"))
}

/// `open(path, "rb").read()`.
pub fn read_bytes(path: &str) -> R<Vec<u8>> {
    std::fs::read(path).map_err(|e| io_fatal("cannot read", path, e))
}

/// `data.decode("utf-8", "replace")` (no newline translation: binary read).
pub fn lossy_text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `open(path).read()`: strict UTF-8 and universal newlines (`\r\n` and a
/// lone `\r` read as `\n`).
pub fn read_text(path: &str) -> R<String> {
    let b = read_bytes(path)?;
    let s = String::from_utf8(b).map_err(|e| Error::Fatal(format!("{path}: not UTF-8: {e}")))?;
    Ok(universal_newlines(&s))
}

/// Text-mode newline translation of Python's `open()`.
pub fn universal_newlines(s: &str) -> String {
    if s.contains('\r') {
        s.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        s.to_string()
    }
}

/// `open(path, "w").write(text)`.
pub fn write_text(path: &str, text: &str) -> R<()> {
    std::fs::write(path, text).map_err(|e| io_fatal("cannot write", path, e))
}

/// `os.path.basename`.
pub fn basename(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// `os.path.dirname`.
pub fn dirname(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => {
            let head = &p[..i + 1];
            let t = head.trim_end_matches('/');
            if t.is_empty() {
                head
            } else {
                t
            }
        }
        None => "",
    }
}

/// `os.path.join(a, b)`.
pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// `os.path.normpath` (POSIX).
pub fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let initial = if p.starts_with("//") && !p.starts_with("///") {
        "//"
    } else if p.starts_with('/') {
        "/"
    } else {
        ""
    };
    let mut comps: Vec<&str> = Vec::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial.is_empty() && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(c);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let s = format!("{initial}{}", comps.join("/"));
    if s.is_empty() {
        ".".into()
    } else {
        s
    }
}

/// The current directory as `os.getcwd()` reports it.
pub fn getcwd() -> String {
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| ".".into())
}

/// `os.path.abspath`.
pub fn abspath(p: &str) -> String {
    if p.starts_with('/') {
        normpath(p)
    } else {
        normpath(&join(&getcwd(), p))
    }
}

/// `os.path.relpath(path, start)`.
pub fn relpath(path: &str, start: &str) -> String {
    let a: Vec<String> = abspath(start).split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    let b: Vec<String> = abspath(path).split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    let i = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".into(); a.len() - i];
    parts.extend(b[i..].iter().cloned());
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// `os.path.exists`.
pub fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

/// `os.path.isdir`.
pub fn isdir(p: &str) -> bool {
    Path::new(p).is_dir()
}

/* ---- fnmatch ------------------------------------------------------------------ */

/// A `[...]` set as `fnmatch.translate` rewrites it: returns the Python
/// class body (without the brackets) or None for "never matches"/"any".
fn fnmatch_set(pat: &[char], mut i: usize, j: usize) -> String {
    let s = |a: usize, b: usize| pat[a..b].iter().collect::<String>();
    let mut stuff = s(i, j);
    if !stuff.contains('-') {
        stuff = stuff.replace('\\', "\\\\");
    } else {
        let mut chunks: Vec<String> = Vec::new();
        let mut k = if pat[i] == '!' { i + 2 } else { i + 1 };
        loop {
            // pat.find('-', k, j)
            let found = if k < j { (k..j).find(|&x| pat[x] == '-') } else { None };
            match found {
                None => break,
                Some(f) => {
                    chunks.push(s(i, f));
                    i = f + 1;
                    k = f + 3;
                }
            }
        }
        let chunk = if i < j { s(i, j) } else { String::new() };
        if !chunk.is_empty() {
            chunks.push(chunk);
        } else if let Some(last) = chunks.last_mut() {
            last.push('-');
        }
        // Remove empty ranges -- invalid in RE.
        let mut k = chunks.len();
        while k > 1 {
            k -= 1;
            let prev_last = chunks[k - 1].chars().last();
            let cur_first = chunks[k].chars().next();
            if let (Some(p), Some(c)) = (prev_last, cur_first) {
                if p > c {
                    let mut merged: String = {
                        let mut v: Vec<char> = chunks[k - 1].chars().collect();
                        v.pop();
                        v.into_iter().collect()
                    };
                    merged.extend(chunks[k].chars().skip(1));
                    chunks[k - 1] = merged;
                    chunks.remove(k);
                }
            }
        }
        stuff = chunks.iter().map(|c| c.replace('\\', "\\\\").replace('-', "\\-")).collect::<Vec<_>>().join("-");
    }
    // Escape set operations (&&, ~~ and ||).
    let mut esc = String::new();
    for c in stuff.chars() {
        if matches!(c, '&' | '~' | '|') {
            esc.push('\\');
        }
        esc.push(c);
    }
    esc
}

/// A Python regex character class body -> an equivalent regex-crate class.
fn py_class_to_rust(body: &str) -> String {
    let cs: Vec<char> = body.chars().collect();
    let mut i = 0;
    let mut neg = false;
    if cs.first() == Some(&'^') {
        neg = true;
        i = 1;
    }
    let start = i;
    // (lo, hi) items
    let mut items: Vec<(char, char)> = Vec::new();
    let read = |i: &mut usize| -> Option<char> {
        let c = *cs.get(*i)?;
        *i += 1;
        if c == '\\' {
            let n = *cs.get(*i)?;
            *i += 1;
            Some(n)
        } else {
            Some(c)
        }
    };
    while i < cs.len() {
        if cs[i] == ']' && i != start {
            break;
        }
        let lo = match read(&mut i) {
            Some(c) => c,
            None => break,
        };
        if i < cs.len() && cs[i] == '-' && i + 1 < cs.len() {
            let mut k = i + 1;
            if let Some(hi) = read(&mut k) {
                i = k;
                items.push((lo, hi));
                continue;
            }
        }
        items.push((lo, lo));
    }
    let mut out = String::from("[");
    if neg {
        out.push('^');
    }
    for (lo, hi) in items {
        if lo == hi {
            out.push_str(&format!("\\x{{{:x}}}", lo as u32));
        } else {
            out.push_str(&format!("\\x{{{:x}}}-\\x{{{:x}}}", lo as u32, hi as u32));
        }
    }
    out.push(']');
    out
}

/// `fnmatch.fnmatch(name, pat)` on POSIX (case-sensitive, `*` crosses `/`).
pub fn fnmatch(name: &str, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n = p.len();
    let mut out = String::from(r"(?s)\A");
    let mut i = 0;
    while i < n {
        let c = p[i];
        i += 1;
        match c {
            '*' => {
                while i < n && p[i] == '*' {
                    i += 1;
                }
                out.push_str(".*");
            }
            '?' => out.push('.'),
            '[' => {
                let mut j = i;
                if j < n && p[j] == '!' {
                    j += 1;
                }
                if j < n && p[j] == ']' {
                    j += 1;
                }
                while j < n && p[j] != ']' {
                    j += 1;
                }
                if j >= n {
                    out.push_str(r"\[");
                } else {
                    let stuff = fnmatch_set(&p, i, j);
                    i = j + 1;
                    if stuff.is_empty() {
                        return false; // (?!): never matches
                    } else if stuff == "!" {
                        out.push('.');
                    } else {
                        let body = if let Some(rest) = stuff.strip_prefix('!') {
                            format!("^{rest}")
                        } else if stuff.starts_with('^') || stuff.starts_with('[') {
                            format!("\\{stuff}")
                        } else {
                            stuff
                        };
                        out.push_str(&py_class_to_rust(&body));
                    }
                }
            }
            c => out.push_str(&esc(&c.to_string())),
        }
    }
    out.push_str(r"\z");
    Regex::new(&out).map(|r| r.is_match(name)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_matches_python_isspace() {
        assert_eq!(strip("\x1c a \x1f\n"), "a");
        assert_eq!(strip("\u{a0}x\u{85}"), "x");
    }

    #[test]
    fn widened_space_class() {
        assert!(re(r"^a\sb$").is_match("a\x1db"));
        assert!(re(r"^[\[\]\s]+$").is_match("[\x1c]"));
    }

    #[test]
    fn int_parsing() {
        assert_eq!(pint("0012").unwrap(), 12);
        assert_eq!(pint("\u{663}\u{661}").unwrap(), 31); // Arabic-Indic 3 1
        assert_eq!(pint("\u{1d7d9}").unwrap(), 1); // MATHEMATICAL DOUBLE-STRUCK DIGIT ONE
        assert_eq!(py_int_str("0x1F", true).unwrap(), 31);
        assert_eq!(py_int_str(" -0x10 ", true).unwrap(), -16);
        assert!(py_int_str("0123", true).is_err());
        assert_eq!(py_int_str("000", true).unwrap(), 0);
        assert_eq!(py_int_str("1_000", false).unwrap(), 1000);
        assert!(py_int_str("1__0", false).is_err());
        assert_eq!(hex(-5, true), "-0x5");
    }

    #[test]
    fn paths() {
        assert_eq!(normpath("/a/b/../c/./d/"), "/a/c/d");
        assert_eq!(normpath("../x/.."), "..");
        assert_eq!(normpath("//a//b"), "//a/b");
        assert_eq!(relpath("/r/tools/shaders/x.frag", "/r"), "tools/shaders/x.frag");
        assert_eq!(relpath("/a/b", "/a/c/d"), "../../b");
        assert_eq!(dirname("/a/b/"), "/a/b");
        assert_eq!(dirname("/x"), "/");
        assert_eq!(basename("a/b.frag"), "b.frag");
    }

    #[test]
    fn fnmatch_like_python() {
        assert!(fnmatch("/r/tools/shaders/qt3d/a.frag", "/r/tools/shaders/qt3d/*.frag"));
        assert!(fnmatch("/r/qt/sub/x.vert", "/r/qt/*"));
        assert!(!fnmatch("/r/qt3d/x.vert", "/r/qt/*"));
        assert!(fnmatch("a.frag", "?.fr[a-c]g"));
        assert!(fnmatch("a!", "a[!b]"));
        assert!(!fnmatch("ab", "a[!b]"));
        assert!(fnmatch("a]", "a[]]"));
        assert!(fnmatch("a[", "a["));
        assert!(fnmatch("a-", "a[z-a-]"));
        assert!(!fnmatch("ab", "a[z-a]"));
        assert!(fnmatch("a&", "a[&]"));
        assert!(fnmatch("a^", "a[^]"));
    }

    #[test]
    fn repr_quotes() {
        assert_eq!(repr("foo"), "'foo'");
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a\\b\n"), "'a\\\\b\\n'");
    }
}
