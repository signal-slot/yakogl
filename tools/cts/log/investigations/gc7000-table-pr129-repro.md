# PR #129 GC7000 table: does this farm's etna_shc reproduce the 218 new rows?

Date: 2026-10-12. Item `table-repro` of ccfarm plan #140.

**Yes.** Replaying the PR's own method on main's table with an etna_shc built
here from the documented sources gives all 218 new rows, the replaced
`es300/texsize.frag` row and every older row byte for byte. The only text that
differs from the PR's file is the toolchain hash in two header lines. A full
regeneration (no `--append`) does not reproduce the table, as the README says
for any table not built by one binary; that is not a finding about the PR.

## Commits compared

| name | commit |
| --- | --- |
| PR #129 head (`origin/work/int-pr31`) | `419b015` (table content unchanged since `d6951f1c787`) |
| `origin/main` | `2fe4b87` |
| merge base of the PR's table commits | `136659c206c6` |

Table commits in the PR: `7898ab09e4ac` (218 rows appended, texsize replaced),
`d6951f1c787` (review fixes: gl_PointCoord rows), then `419b015` (docs/tests
only).

## Toolchain

- Mesa **26.2.1**, release tarball (no git commit), cached as
  `mesa-src/mesa.tar.xz` in the deps directory, sha256
  `c47e81bddc4760360a41ac3c5acec38acb81f9d750ecef47e7f3adc7021a4442`.
- glslang `11:16.4.0` (`glslangValidator --version`), the pin in the table header.
- Build, as tools/vivante/README.md documents, once per side, in a private
  unpack of the tarball (`table-repro/mesa-main`, `table-repro/mesa-pr`):
  ```sh
  git show <ref>:tools/vivante/etna_shc.c > src/gallium/drivers/etnaviv/etna_shc.c
  patch -p1 < <ref>:tools/vivante/mesa-etna_shc.patch
  meson setup build-etnaviv -Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dplatforms= \
      -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dllvm=disabled -Dtools=
  ninja -C build-etnaviv src/gallium/drivers/etnaviv/etna_shc
  ```
  Both built without errors (a few minutes). sha256 of the results:

  | etna_shc | sha256 |
  | --- | --- |
  | built from the PR's `etna_shc.c` (fresh) | `18619d1de5fb2f3606e259d77fa1662f52e1d158868ba761086db4053650a46a` |
  | built from main's `etna_shc.c` (fresh) | `f8fc5b45909cdd58afc491e3dd290a5cf0404eeb323ce58e42e6ddb916aeaa86` |
  | already cached on the farm (`mesa-src/build-etnaviv`) | `1d52afefcb16bbcde044c6e5434f9f00839022eb40e098837a4606494ddc43d2` |

  The PR header names `1d52afef...` for the 218 appended rows, so "another
  etna_shc build" is the farm's cached binary, byte for byte. The cached tree's
  `etna_shc.c` carries the PR's extra 25 lines (gl_PointCoord as an input,
  `nir_lower_io_array_vars_to_elements_no_indirects`). Main's older rows name
  `e224bebc...` (the laptop's) and `a22662e5...`.

## Method: the PR's own steps

The header of the PR table lists how it was made: `--append` of 218 shaders
by `1d52afef`, `--replace` of `es300/texsize.frag`, `--replace` of four
gl_PointCoord rows. The replay (`table-repro/replay.py`) starts from main's
table (330 rows) in a worktree of the PR (its generator and sources), runs
`gen_shader_table.py --append` with the 218 names from the PR's header, then
`--replace es300/texsize.frag`, once with each binary. Then it compares with
the PR's `tests/reference/vivante_blobs.rs`.

## Results

| check | cached `1d52afef` | fresh PR build `18619d1d` |
| --- | --- | --- |
| `--append` of the 218 | ok, "kept 330 committed rows byte for byte, appended 218" | same |
| `--replace es300/texsize.frag` | ok, 547 other rows kept | same |
| `--replace` of the 4 gl_PointCoord rows | no change (see below) | same |
| rows in result / in PR table | 548 / 548 | 548 / 548 |
| rows identical (name, sha256, code, reflection) | **548** | **548** |
| rows differing / only in one table | 0 / 0 | 0 / 0 |
| line diff against the PR file | 5 lines (the pointcoord `--replace` header) | 9 diffstat lines (those 5 missing, plus the 2 header lines naming the toolchain hash, removed and re-added) |

- **New rows vs the PR table:** 218 identical, 0 differing. No instruction
  words differ, so no decoding with `tools/gc7000-isa/decode.py` was needed.
- **Existing rows vs the PR table:** 330 main rows: 329 unchanged, 1
  (`es300/texsize.frag`) replaced and re-keyed because its source text changed;
  0 changed otherwise. Against `origin/main`: 218 new + 1 re-keyed = 219 keys
  only in the PR; 1 key only in main (the old texsize digest).
- **The four-row `--replace` is a no-op in the replay,** because the replay
  appends with the PR's `etna_shc.c` which already has the gl_PointCoord fix;
  the PR appended with an earlier tool state and replaced those rows
  afterwards. The end state is the same, and the five missing header lines are
  that replace's listing.
- **Caveat on the binary:** the fresh build gives different bytes than the
  cached one (different paths and flags), yet the same rows. So the table
  depends on the sources and the pins, not on the binary's exact bytes. The
  header hash will differ for anyone who rebuilds; do not treat it as a check.

## A full regeneration is not the test

`gen_shader_table.py` with no arguments on the PR tree with the cached binary
printed "reproduced 56 of 548 committed rows byte for byte (changed 487,
removed 5, added 79)" and refused to overwrite the table. This is the README's
point: a table built by `--append`/`--replace` over several binaries cannot be
rebuilt in one run, and a rebuild is a new reference. It also printed many
`FAIL` lines for shaders outside the table (nested structs, SPIR-V failures).
The PR does not claim a full rebuild.

## Not done

- A full regeneration on `origin/main`'s tree ran only with the farm's cached
  `etna_shc` (`1d52afef`, built from the PR's `etna_shc.c`), not with main's
  own build (the `ETNA_SHC` override was not allowed): "reproduced 21 of 330
  committed rows byte for byte (changed 304, removed 5, added 191)", refused as
  a new reference. Same conclusion as for the PR: a full rebuild is not
  reproducible on any current binary, which is why the PR used `--append`.
- The generator wrote only into the throwaway worktree. `tests/reference/vivante_blobs.rs`
  in this repository is untouched and no regenerated table is committed.
