<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000 depth resolve: a faster unorm24-to-float, checked over all 2^24 inputs

`docs/notes/gc-depth-resolve.md` leaves one cost in the CPU depth resolve
(`ds_cache::resolve_depth`, once a frame in glmark2 shadow and refract): the
f64 divide per texel in `ds_retention::unorm24_depth`,
`((word & 0xff_ffff) as f64 / 16_777_215.0) as f32`. A replacement must give
the same bits for every input, because the depth texture's floats feed
shadow compares. `tests/gc_unorm24_exhaustive.rs` settles that: it compares
every candidate with the reference for each `v` in `0..2^24` (about a second
at the test profile's opt-level 3), asserts the reference equals the crate's
public `yakogl::ds_retention::unorm24_depth` (reachable; `src` is not
edited), and fails if a candidate's verdict below changes.

## Which candidates are bit-exact

| Candidate | Bit-exact | Mismatches of 2^24 | First counterexamples (v: want, got) |
|---|---|---|---|
| f64 multiply by the f64 reciprocal `1.0 / 16_777_215.0` | yes | 0 | |
| f32 multiply by the f32 reciprocal | **no** | 8 388 606 | 3: `0x34400001`, `0x34400002`; 6: `0x34c00001`, `0x34c00002`; 7: `0x34e00001`, `0x34e00002` |
| f32 reciprocal, then one fma residual correction (`r = fma(-q, D, x)`, `fma(r, rcp, q)`) | yes | 0 | |
| f64 reciprocal, then the same fma correction, rounded to f32 last | yes | 0 | |
| two 4096-entry f64 tables (`hi * 4096 / D`, `lo / D`), added in f64 | yes | 0 | |
| the full 2^24-entry f32 table (64 MiB, built from the reference) | yes | 0 | |

The f32 reciprocal alone is wrong for half of all inputs, by one ulp (it
rounds the reciprocal and the product, two roundings); the fma step repairs
every one of them. The f64 reciprocal product has an error of at most about
2^-53 relative, far below the f32 rounding step, so double rounding never
changes the f32 result for these inputs: exact, with no correction.

## Timings (host, 1024 x 1024 surface, best of 30)

`time_1024_square_conversions` converts a 1M-word D24S8 surface the way
`resolve_depth` reads it (`word >> 8`, then the conversion). Run on the
x86-64 build host at opt-level 3, no `target-cpu`:

| Candidate | ms |
|---|---|
| reference, f64 divide | 0.22 |
| f64 multiply by reciprocal | 0.37 |
| f32 multiply by reciprocal (not exact) | 0.23 |
| f32 reciprocal + fma correction | 4.6 |
| f64 reciprocal + fma correction | 4.6 |
| split lookup 2 x 4096 | 0.96 |
| full 2^24 table | 11.7 |

Read these with care. The host's vector divide (`divpd`) is pipelined, so
the divide is already as fast as the multiply here; the loop is vectorised
for the reference and the multiplies alike. The fma rows are slow only
because the build has no `+fma` target feature, so `mul_add` is a libm call;
on aarch64 (the i.MX8MP's Cortex-A53) `fmadd` is one instruction. The full
table is slower than computing, since it is 64 MiB of cache misses. **The
host does not show a win for any candidate, and it cannot predict the
board**: the Cortex-A53's double-precision divide is, by its published
timings, many cycles per texel and not fully pipelined, while a multiply is
pipelined. That is an expectation, not a measurement: the gain, if any, is
for the board session to measure.

## Recommendation for the fix item

Replace the divide with the f64 reciprocal multiply:

```rust
const RCP: f64 = 1.0 / 16_777_215.0;
pub fn unorm24_depth(word: u32) -> f32 {
    ((word & 0xff_ffff) as f64 * RCP) as f32
}
```

It is exact over all 2^24 inputs (the test), needs no table and no fma, and
vectorises. Do not use the f32-only multiply (one-ulp errors on half the
inputs). The fma forms are exact but cost more where fma is not a target
feature and gain nothing over the f64 product; the tables are exact but
slower. The fix item edits `src/ds_retention.rs` (not `src/vivante`; the
function lives in `src/ds_retention.rs` and `src/pvr/backend.rs` calls it too, with the
same output), keeps `tests/gc_unorm24_exhaustive.rs` (change its reference
copy only if the formula's meaning changes) and measures the 1024 x 1024
resolve on the board before and after, since the host cannot show a gain.

## Status: the multiply is the crate's

`ds_retention::unorm24_depth` now multiplies by the f64 constant
`1.0 / 16_777_215.0` (commit "ds_retention: unorm24_depth multiplies by the
f64 reciprocal"; same signature, same bits, so `ds_cache::resolve_depth` and
`pvr/backend.rs` output is unchanged). `tests/gc_unorm24_exhaustive.rs` keeps
its own divide as the reference and still asserts the crate function equals it
for every input; it passes after the change.

1024 x 1024 conversion time, `time_1024_square_conversions`, best of 30,
x86-64 host, release build (`--features host-test,gpu-vivante`). The crate is
not timed directly; the divide row is the code before, the multiply row the
code after:

| | ms |
|---|---|
| before (f64 divide) | 0.243 |
| after (f64 reciprocal multiply) | 0.258 |

As predicted above, the host shows no gain (the difference is run-to-run
noise; the divide is pipelined here). The shadow/refract host harness
before/after was not run for this change; the board gain, if any, is for the
board session.
