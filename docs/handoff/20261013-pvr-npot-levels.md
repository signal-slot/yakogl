# pvr-npot-levels, 2026-10-13, item am62p-es2-r1c

## Open work
- Stage 1 residual risk "the PowerVR TPU's level arithmetic for
  `2d.npot_size`" (`docs/es2-cts-failures.md` H.5): host-pinned by
  `tests/pvr_npot_levels.rs` (5 tests, all pass; code and Mesa's layout rules
  agree, no bug found). Record: `docs/investigations/pvr-npot-levels.md`.
  What stays open is the board: the AM62P final tree (`am62p-es2-final-a`)
  must still show `texture.completeness.2d.npot_size` as Pass, and glmark2
  terrain must still render.
- The old scope of `am62p-es2-r1c` (images 8-17, ms4, gl_test) is not
  repeated; its record is `tools/cts/log/board-sessions/am62p-es2-r1c.md`.
- Follow-up for another item (a `src/` change): `build_tex_desc` is private,
  so the texture words are derived in the test from the bind, not read from
  the code. Exposing it under `host-test` would let the test compare the
  real words.
