<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# es3-attrib-location, 2026-10-13, item oq-v3d

## State of main
`glBindAttribLocation` no longer moves a vertex input that has
`layout(location = N)` (ES 3.0 2.12.3, GLSL ES 3.00 4.3.8.1).
`src/gl/program.rs`: `ExplicitAttribs::scan` reads the qualifiers from the
vertex shader source (only `#version 300 es`; ES 2.0 and `#version 450`
fixtures read nothing, so `report-es2` answers do not change),
`attrib_layout` and `build_attrib_indices` give such a name its qualifier
and skip its binding, and `link_attrib_layout` compiles the vertex shader
again with the decided layout when it differs from the compile's.
`tests/es3_attrib_location.rs` reproduces dEQP `attribute_location.mixed`
and `mixed_time.{pre_attach,pre_link,reattach,relink,post_link}` through the
GL API and passes. Expected effect on AM62P: the 28 `mixed` and `mixed_time`
failures (Appendix A, task T4). Not run on a board.

## Open work
- Still open: `attribute_location.mixed_max_attributes.*` (20 AM62P cases).
  The GL core cannot fix it alone. When the compile puts two inputs on one
  location, the record has one row for that location
  (`src/shaderc/online/powervr/abi.rs`, the `find(|d| d.location == ...)`
  merge), so the second input is gone before the link sees it. The link
  therefore keeps the compile's placement for such shaders
  (`layout_differs` returns false when an active input has no row).
  **`src/shaderc/online/reflect.rs` must change** (held by tf-infolog when
  this was written). In `reflect()`, `StorageQualifier::Attribute` branch:
  an input without `layout(location)` must get the lowest run of free
  locations that no `layout(location)` of the shader uses and no earlier
  input took, instead of `attrib_loc` (the counter continuing past the
  previous explicit location). Sketch, proven on a throw-away patch (not
  committed):
  1. before the loop (or per input), collect a bit mask `explicit` of
     `l .. l + locations(base) * array_len` for every top-level attribute
     with `io.location == Some(l)`;
  2. keep a mask `placed` of the locations earlier unqualified inputs took;
  3. for an unqualified input of `n = locations(base) * count` locations:
     start at 0, advance while any of `loc .. loc + n` is in
     `explicit | placed`, then use `loc` and add it to `placed`;
  4. an input with a qualifier keeps `attrib_loc = l` as now.
  For a shader with no qualifier this is the same as the counter, so ES 2.0
  and unqualified ES 3.0 shaders reflect exactly as before.
  With that patch `mixed_max_attributes_vec4` and
  `unqualified_unbound_inputs_avoid_every_qualifier` in
  `tests/es3_attrib_location.rs` pass; remove their `#[ignore]` in the same
  change as the reflect.rs edit and add the reflect.rs unit test for the
  placement.
- Limits of the source scan: a location written through a macro, or
  `#if`-ed out qualifiers, are not seen (the input then counts as
  unqualified). dEQP writes the plain form.

## Next board session
After this lands, re-run `functional.attribute_location.mixed` and
`mixed_time` on verdin-am62p (expect 0 failures there), and
`mixed_max_attributes` after the reflect.rs change.
