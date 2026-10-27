// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools etna-dis` - disassemble a Vivante GC7000 shader binary.
//!
//! The human-readable side of the online compiler's GC7000 decoder
//! (`src/shaderc/online/vivante/`): the same tables, the same parse, printed.
//! It takes a raw binary of 128-bit instructions, or the words on the command
//! line, so a shader dumped from the driver or pasted from a log can be read.
//!
//!     yakogl-tools etna-dis <file>                 a raw GC7000 binary
//!     yakogl-tools etna-dis --hex '0x... 0x... ..' words, any order of 4
//!
//! The GC7000 table of blobs is only compiled into a `gpu-vivante` build, so
//! this reads its input rather than the table; `cargo test --features
//! host-test,gpu-vivante,shaderc-online vivante::` is what walks the table.

use crate::{Error, R};
use yakogl::shaderc::online::vivante::{decode, listing};

const USAGE: &str = "usage: yakogl-tools etna-dis [<file> | --hex '<words>']";

fn parse_hex(s: &str) -> R<Vec<u32>> {
    let mut out = Vec::new();
    for tok in s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()) {
        let t = tok.trim_start_matches("0x").trim_start_matches("0X");
        out.push(u32::from_str_radix(t, 16).map_err(|_| Error::Exit(format!("'{tok}' is not a 32-bit hex word")))?);
    }
    Ok(out)
}

pub fn main(args: &[String]) -> R<()> {
    let mut words: Option<Vec<u32>> = None;
    let mut label = String::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--hex" => {
                let s = it.next().ok_or_else(|| Error::Usage(USAGE.into()))?;
                words = Some(parse_hex(s)?);
                label = "--hex".into();
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => {
                let b = std::fs::read(other).map_err(|e| Error::Fatal(format!("{other}: {e}")))?;
                if b.len() % 4 != 0 {
                    return Err(Error::Exit(format!("{other}: {} bytes is not whole words", b.len())));
                }
                words =
                    Some(b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
                label = other.to_string();
            }
            other => return Err(Error::Usage(format!("{USAGE}\n\nunknown option '{other}'"))),
        }
    }
    let words = words.ok_or_else(|| Error::Usage(USAGE.into()))?;
    if words.len() % 4 != 0 {
        return Err(Error::Exit(format!("{} words is not a whole number of 128-bit instructions", words.len())));
    }

    println!("{label}: {} instructions", words.len() / 4);
    let mut text = String::new();
    for (i, dw) in words.as_chunks::<4>().0.iter().enumerate() {
        let quad = [dw[0], dw[1], dw[2], dw[3]];
        match decode::decode_instr(&quad) {
            Ok(instr) => listing::instr_text(&mut text, i, &instr).map_err(|e| Error::Fatal(e.to_string()))?,
            Err(e) => {
                print!("{text}");
                return Err(Error::Exit(format!("instruction {i} ({quad:08x?}): {e:?}")));
            }
        }
    }
    print!("{text}");
    Ok(())
}
