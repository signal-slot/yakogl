// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools usc-dis` - disassemble a PowerVR USC binary.
//!
//! It is the human-readable side of the online compiler's decoder
//! (`src/shaderc/online/`): the same tables, the same parse, printed. Use it
//! to read what a blob of the offline table actually contains, and to compare
//! a group against `pco_shc`'s own listing of the same shader while the
//! decoder is being extended.
//!
//!     yakogl-tools usc-dis --table <source-name> [--nth N]   a row of the offline table
//!     yakogl-tools usc-dis --index <N>                       a row by table index
//!     yakogl-tools usc-dis <file>                            a raw USC binary
//!     yakogl-tools usc-dis --list                            what the table holds

use crate::{Error, R};
use yakogl::shaderc::online::powervr::{decode, listing};
use yakogl::shaderc::table::SHC_BLOB_TABLE;

const USAGE: &str = "usage: yakogl-tools usc-dis [--table <name> [--nth N] | --index <N> | <file>]
                            [--list]";

/// The USC of one table row, as bytes (the table stores dwords).
fn row_bytes(i: usize) -> Vec<u8> {
    let b = &SHC_BLOB_TABLE[i];
    // SAFETY: `usc` points at a `[u32; usc_dw]` static of the generated table.
    let dw = unsafe { core::slice::from_raw_parts(b.shader.usc, b.shader.usc_dw as usize) };
    dw.iter().flat_map(|w| w.to_le_bytes()).collect()
}

pub fn main(args: &[String]) -> R<()> {
    let mut table: Option<String> = None;
    let mut index: Option<usize> = None;
    let mut nth = 0usize;
    let mut file: Option<String> = None;
    let mut list = false;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--table" => table = Some(it.next().ok_or_else(|| Error::Usage(USAGE.into()))?.clone()),
            "--nth" => nth = it.next().and_then(|s| s.parse().ok()).ok_or_else(|| Error::Usage(USAGE.into()))?,
            "--index" => {
                index = Some(it.next().and_then(|s| s.parse().ok()).ok_or_else(|| Error::Usage(USAGE.into()))?)
            }
            "--list" => list = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other if !other.starts_with('-') => file = Some(other.to_string()),
            other => return Err(Error::Usage(format!("{USAGE}\n\nunknown option '{other}'"))),
        }
    }

    if list {
        for (i, b) in SHC_BLOB_TABLE.iter().enumerate() {
            println!("{i:5} {:<40} blend={:#x} {} dwords", b.source_name(), b.blend_key, b.shader.usc_dw);
        }
        return Ok(());
    }

    let (label, bytes) = if let Some(i) = index {
        if i >= SHC_BLOB_TABLE.len() {
            return Err(Error::Exit(format!("index {i} is past the table's {} rows", SHC_BLOB_TABLE.len())));
        }
        (format!("{} [{i}]", SHC_BLOB_TABLE[i].source_name()), row_bytes(i))
    } else if let Some(name) = table {
        let hits: Vec<usize> = (0..SHC_BLOB_TABLE.len()).filter(|&i| SHC_BLOB_TABLE[i].source_name() == name).collect();
        let i = *hits
            .get(nth)
            .ok_or_else(|| Error::Exit(format!("'{name}': {} rows in the table, no {nth}", hits.len())))?;
        (format!("{name} [{i}], blend {:#x}", SHC_BLOB_TABLE[i].blend_key), row_bytes(i))
    } else if let Some(p) = file {
        let b = std::fs::read(&p).map_err(|e| Error::Fatal(format!("{p}: {e}")))?;
        (p, b)
    } else {
        return Err(Error::Usage(USAGE.into()));
    };

    println!("{label}: {} bytes", bytes.len());
    let mut text = String::new();
    let mut off = 0;
    let mut groups = 0;
    while off < bytes.len() {
        match decode::decode_group(&bytes, off) {
            Ok(g) => {
                listing::group_text(&mut text, off, &g).map_err(|e| Error::Fatal(e.to_string()))?;
                off += g.total;
                groups += 1;
            }
            Err(e) => {
                print!("{text}");
                return Err(Error::Exit(format!("decode failed at +{off:#x}: {e:?}")));
            }
        }
    }
    print!("{text}");
    println!("{groups} instruction groups");
    Ok(())
}
