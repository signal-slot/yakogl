<!--
SPDX-License-Identifier: MIT
-->

# Which hwdb row the i.MX8MP's GC7000UL is: customer id 0 or 0x11

Investigation `gc-customer-id` of plan #328. Host only: no board was run and
no code or patch changed. Code is cited as `path:line` at `0a922ae2`. The
hwdb is cited as `H:line`, NXP's `src/etnaviv/hwdb/nxp/gc_feature_database.h`
in Mesa 26.2.1 (the deps cache's `mesa-pan/mesa-26.2.1`, the tree of
`tools/vivante/README.md`).

## Answer

**Unsettled.** Nothing in the tree, in a record or in a test fixture carries
the value the board's `VIVS_HI_CHIP_CUSTOMER_ID` (0x30) returns. Both numbers
in use are assumptions:

- 0x11 is the customer id of the only formal row that has the board's
  revision, product and eco (`H:18845`). The code takes it by reading the
  hwdb, not the board.
- 0 is what `tools/vivante/mesa-cs-golden.patch:84` passes, under the comment
  "as the Verdin iMX8M Plus reports it" (`:70`). That comment has no source:
  the line arrived whole in the release commit `381b0f09` and no note,
  record or issue text in the tree has a customer id read from a board
  (searched: `customer` in docs, tools, tests, `tools/cts/`; `git log -G` on
  the `customer 0x.. eco` console format finds only `f862e8e7`, the stream
  diff that asked the question).

The two ids are not equivalent, and the board's own lookups cannot tell them
apart (below). Only a read of register 0x30 settles it ("The exact read").

## The evidence, piece by piece

### 1. Where the register is read, and what keeps it

- `driver/rust/src/gpu/vivante/hw.rs:176` reads it
  (`id.customer_id = rd(m, regs, VIVS_HI_CHIP_CUSTOMER_ID)`, offset 0x30 at
  `driver/rust/src/gpu/vivante/regs.rs:61`) and `hw.rs:202-209` logs it:
  `identity ...: GC7000 rev 0x6204 product 0x70003 customer 0x.. eco ..`.
  That log line goes to the console (`ylog!`, level Inf).
- `driver/samples/zephyr/gc_clear/src/main.c:152` and
  `gc_fence/src/main.c:92` print `customer 0x%x` as well, again on the
  console.
- The Verdin bring-up of 2026-09-20 (`docs/notes/imx8mp-gc-bringup.md:212`)
  recorded the verdict only: "GC7000 rev 0x6204". The SNVS record has no
  identity word: `gc_result.h:112` names stage 0x38 "the identity read
  entered" and 0x39 "the GPU reset entered", and neither carries a value.
- Nothing in our lookup uses it. `hw.rs:37-47,77` keys a row on customer
  only where the row gives one, and both rows give `None`
  (`hw.rs:90,120`); the test says so (`driver/rust/src/gpu/vivante/tests.rs:478`,
  it pokes 0x15 into the register of both parts and still identifies).
  `driver/docs/vivante-kernel-rust.md:229` lists "customer and eco ids not
  recorded" for the product-id match. `src/vivante/profile.rs:32-37` says the
  lookup does not match eco and customer and that no board's customer id was
  read; the only readers of `ChipProfile::customer` are the two assertions in
  `tests/vivante_profile.rs:22,40`.

So the passing board runs (every `gc_identify`, gl_test, glmark2 run) prove
only model 0x7000, revision 0x6204, product 0x70003, which select neither
row on their own.

### 2. Where 0x11 comes from

`src/vivante/profile.rs:107` (`customer: 0x11 // :18845`),
`src/vivante/backend.rs:768-771` (a doc comment: "CustomerID 0x11, both named
`gc7000ULN_v123`, at :18841 and :19735"), `docs/es2-cts-failures.md:88`,
`docs/es3/backend-caps.md:91`, `docs/notes/gc-tiled-targets.md:100`,
`tools/vivante/hwdb_compare.py:32` (`NXP_PINNED = (0x7000, 0x6204, 0x70003, 0,
0x11)`), `tools/vivante/testdata/hwdb-compare-st-6205.txt:8` and
`tests/vivante_npot.rs:24,65,73`. Every one of them is the hwdb row (`H:18845`
and, for eco 1, `H:19739`) read as "the part's ids". `docs/notes/gc-msaa-addresses.md:50`
and `docs/es3/srgb.md:467` give model, revision and product only. None is a
board read. Those docs are not wrong about what the row says; they are wrong
only if they are taken as saying what the board reports.

### 3. Where 0 comes from

`mesa-cs-golden.patch:84` (`[ETNAVIV_PARAM_GPU_CUSTOMER_ID] = 0x0`) plus the
shim's `C11GC7000UL` entry (the same with 0x11, `docs/notes/gc-slow-terrain-stream-diff.md:109`,
in the Mesa tree only). The ST entry in the same patch (`:101`) says "the
customer id was not read; 0x15 is the etnaviv hwdb's value for this row" - the
same method for a second part. The GC7000UL entry was written before that
convention and has no comment of the kind. It is most likely a default of
"not set" that was then described as a read; this is a guess and the note
does not rely on it.

### 4. What Mesa's lookup does with each value

`H`'s `gcQueryFeatureDB` is `hwdb.h.py:43-82`: first the formal rows by exact
ChipID, ChipVersion, ProductID, EcoID and CustomerID, then the informal rows
(`FormalRelease` 0) by ChipVersion & 0xfff0 and exact the rest. Mesa passes
the kernel's ids since DRM 1.4 (`etna_hwdb.c:31-32`). The NXP file has
four rows for model 0x7000, product 0x70003:

| row (comment) | revision | eco | customer | line |
|---|---|---|---|---|
| `gc7000UL_6200` | 0x6202 | 0 | 0 | `H:15265` |
| `gc7000ULN_v122` | 0x6203 | 0 | 4 | `H:17947` |
| `gc7000ULN_v123` | 0x6204 | 0 | 0x11 | `H:18841` |
| `gc7000ULN_v123` | 0x6204 | 1 | 0x11 | `H:19735` |

All have `FormalRelease` 0 (informal). For the board's revision 0x6204,
product 0x70003, eco 0:

- customer 0x11 matches `H:18841` (also `H:19735` for eco 1; the two are
  identical apart from the EcoID, `gc-tiled-targets.md:100`);
- customer 0 matches `H:15265` only, through the informal mask (0x6204 &
  0xfff0 == 0x6202 & 0xfff0). Our `hw::DB` does the same masking for the
  row it keeps (`hw.rs:82-90`, from `vivante-kernel-rust.md:155`);
- any other customer id finds no row, and Mesa then falls back to the
  feature words in `reg_map`, which the patch leaves empty: the shim would
  not work. The patch's own header says the shim "looks the core up", so a
  third value is excluded by the fact that the golden runs.

The kernel side, for context: Linux's own hwdb carries model 0x7000 /
revision 0x6204 with product and customer left open (`~0U`) and a separate
`gc7000UL_6200` row (revision 0x6202, customer 0, Streams 8) -
`docs/notes/pre-study-stm32mp257f-dk.md:160-181` cites
`etnaviv_hwdb.c:72-86,136-140` at the pinned clone. That says Linux itself
wanted to cover a 0x6204 part whose customer id it did not want to name, so
it cannot decide either way. No kernel tree (NXP galcore or mainline
etnaviv) is in the deps cache (`connected-deps` has `mesa-pan`, `glmark2-src`,
the Zephyr and PCO trees), so galcore's own database lookup was not read.
Circumstantial only: NXP wrote a row for ChipRevision 0x6204 with customer
0x11 and none with customer 0, and its i.MX8MP BSP uses galcore, whose
database is this file (`gcQueryFeatureDB` is NXP's function). That makes 0x11
the likelier value for a 0x6204 part; it is not evidence of what is on the
silicon.

### 5. The rows, field by field

890 fields per row. Row `H:15265` (customer 0, revision 0x6202) against
`H:18841` (customer 0x11, revision 0x6204) differ in 31:

| field | 6202 row | 6204 row | lines (6202 / 6204) |
|---|---|---|---|
| ChipRevision | 0x6202 | 0x6204 | 15266 / 18842 |
| CustomerID | 0 | 0x11 | 15269 / 18845 |
| LocalStorageSize, L1CacheSize, USC_MAX_PAGES | 0x10 | 0x8 | 15279, 15282, 15287 / 18855, 18858, 18863 |
| Streams | 8 | 16 | 15299 / 18875 |
| REG_ZCompression | 1 | 0 | 15380 / 18956 |
| V4Compression | 1 | 0 | 15615 / 19191 |
| VMSAA | 1 | 0 | 15609 / 19185 |
| REG_BugFixes23, TX_YUV_ASSEMBLER_10BIT | 1 | 0 | 15599, 15651 / 19175, 19227 |
| TX_SEAMLESS_CUBE, TX_SNORM_SUPPORT, PE_A8B8G8R8 | 0 | 1 | 15725, 15726, 15737 / 19301, 19302, 19313 |
| TX_FLUSH_L1CACHE, MC_FCCACHE_BYTEMASK, FE_PATCHLIST_FETCH_FIX | 0 | 1 | 15694, 15697, 15700 / 19270, 19273, 19276 |
| SH_ROBUSTNESS_FIX, USC_ATOMIC_FIX2, DE_2D_FAST_CLEAR, IMGLD_WIDTH_LT16_FIX | 0 | 1 | 15735, 15736, 15747, 15775 / 19311, 19312, 19323, 19351 |
| EVIS_NO_{ABSDIFF, BITREPLACE, BOXFILTER, CORDIAC, DP32, FILTER, IADD, SELECTADD} | 0 | 1 | 15853-15860 / 19429-19436 |

The tile-status fields that Mesa's TS code reads are **the same** in both:
`REG_FastClear`, `REG_TileStatus2Bits`, `CACHE128B256BPERLINE`, `REG_MC20`,
`REG_SuperTiled32x32` and `REG_SuperTiledTexture` (`gc-tiled-targets.md:100-112`).
What changes is only compression: `V4Compression` and `REG_ZCompression`
(plus `VMSAA`). In Mesa, colour compression needs v4 or MSAA
(`etnaviv_resource.c:196-199`, `gc-tiled-targets.md`), so the 6202 row turns
`TS_MEM_CONFIG`'s colour (0x80) and depth (0x40) compression on and makes
the in-place resolves necessary (`gc-slow-terrain-stream-diff.md:304`). The
`Streams` value of 8 is not read from the registers by Mesa, which takes it
from the database; our own `hw::DB_IMX8MP` has 8 for the informal row
(`vivante-kernel-rust.md:155-158`) while `profile.rs` has 16
(`profile.rs:114-115` cite `:18875-18876`), so the two already disagree on
this part.

A caveat on the "Streams" and cache-size fields: the board answered all
recorded runs with the stream count our own `hw::DB` took, so those runs
do not exercise the difference.

## What the answer changes

If the board reports **0x11** (the code's assumption):

- `profile.rs`, `backend.rs:768`, the cited docs: unchanged and now backed
  by a read. The comment "neither board's customer id was read"
  (`profile.rs:35`) is replaced by the record.
- `mesa-cs-golden.patch:84` is wrong: it gives Mesa the wrong row. The
  patch's goldens and everything that cites them
  (`gc-slow-stream-diff.md`, `gc-slow-terrain-stream-diff.md`, the
  README's GC7000UL-against-ST table at `tools/vivante/README.md:690-701`)
  describe row 6202. The README's compression finding (`TS_MEM_CONFIG` 0x43c2,
  `COLOR_COMPRESSION` 0x80, `DEPTH_COMPRESSION` 0x40) is then a property of
  the wrong row, **not** of the board. `gc-tiled-wire.md` "Compression, an
  open point" (`:550-557`) closes as: the board has V4Compression 0, our
  "no compression" design is right, and the W8 tests need not mask bits
  0x40, 0x80, 0xf00 and 0x4000. The fix is to use `C11GC7000UL` (or set
  the patch's entry to 0x11), after which the goldens lose their 6 in-place
  resolves a frame and the TS compression, and the TS buffers get larger.
  `ts-golden` owns the patch: it should regenerate on the 0x11 row.
- Ranked difference 9 of `gc-slow-terrain-stream-diff.md` ("new" row,
  `:304`) becomes a shim defect, with no board cost. Fix: new issue 1 below.

If the board reports **0**:

- `profile.rs:107`, `backend.rs:768`, `es2-cts-failures.md:88`,
  `es3/backend-caps.md:91`, `gc-tiled-targets.md:100` and the tests
  `tests/vivante_profile.rs:40`, `tests/vivante_npot.rs` cite the wrong
  row. The part is the informal `gc7000UL_6200` row: `V4Compression` 1,
  `REG_ZCompression` 1, `VMSAA` 1, Streams 8, `LocalStorageSize` /
  `L1CacheSize` / `USC_MAX_PAGES` 0x10, and *no* `TX_SEAMLESS_CUBE`,
  `TX_SNORM_SUPPORT`, `PE_A8B8G8R8`, `FE_PATCHLIST_FETCH_FIX` and the rest of
  the second group. Each of those is an assumption of ours that would then
  be a defect: seamless cubes (`gc7000UL` features cited from `:19049` of
  the 6204 row in `backend.rs:772-779`), SNORM textures, A8B8G8R8 render
  targets. Streams 8 would also match `vivante-kernel-rust.md:155`, where
  `profile.rs` has 16.
- `gc-tiled-wire.md` compression: the board has V4Compression 1, the design
  must program (or at least tolerate) compression, TS tiles being written
  compressed by the GPU, and the W8 masks and the "no compression"
  assumption fail. The README's 0x43c2 golden is then the right one.
- The goldens stay as they are.

Which of the two is true also decides the whole `ts-golden` baseline, so
`ts-golden` should not regenerate until the read is in.

## The exact read

Register `VIVS_HI_CHIP_CUSTOMER_ID`, offset 0x30 from the GC block base
(`regs.rs:61`), read by `hw::identify` at `hw.rs:176` into
`GcIdentity::customer_id`. It is already read on every run; only its report
path is missing. Without a console on the Armadillo-X2, the carriers are, in
order of least change:

1. **A record word.** `gc_result.h:112-113` stage 0x38/0x39 records exist as
   STAGE kind records (`yakogl_record.h:38`), whose w1 is "unused" for 0x38
   and 0x39. After `gc_init`'s `yd_gc_identify` (`gpu_vivante_gc_rust.c:172`),
   a STAGE record with w1 = `customer_id`, w2 = `eco_id` (and a second
   with `product_id`) would carry it into the SNVS record that
   `tools/board.sh x2 run` reads. This is a change to
   `gpu_vivante_gc_rust.c` and `gc_result.*`, which plan #323 owns; a plan
   after #323 does it, or #323 adds the word. I did not check the
   record kinds' layout beyond `yakogl_record.h:6-46`.
2. **A gl_test case** (`driver/tools/pvrlab/gl_test.c`): a case that
   fails unless `gc_identity(dev)->customer_id == 0x11` would turn the
   verdict line into the answer (PASS = 0x11; FAIL = anything else), needing
   no record word; a second case for `== 0` would settle which way a FAIL
   went. Two cases, one boolean each. Existing GC cases do not read it.
3. **GL_RENDERER or the glmark2 PERF page** (`fb-page4` adds a page 4): a
   string carrying the id. Larger change; not preferred.

On the Verdin, a console line (`gc_clear` or `gc_fence` run, `gc_identify`
logged at `hw.rs:202`) is the one-line read, but the Verdin is held by its
owner and the X2 has no console; the same SoC and GPU make the X2's value the
Verdin's. That the X2's value equals the Verdin's is an assumption (same SoC and GPU revision 0x6204; the patch's comment names the Verdin), so a Verdin line, if the owner has one, is a second source.

## What this note does not show

- The register's value, on any board.
- Whether NXP's galcore (which has this hwdb as its feature database) would
  select 6202 or 6204 for the board. The galcore or NXP kernel source is not
  in the deps cache.
- That the patch's `0x0` was not read from a Verdin by someone. The comment
  says so; no record backs it. If a person did read it off a Verdin console,
  the answer is 0 and the code's 0x11 is the error. Asking the owner for the
  log line `identity ...: customer 0x..` of a Verdin boot is the cheapest
  settlement of all.

## New issue (proposed text)

**Title:** `GC7000UL customer id was never read: the Mesa golden uses 0, the code 0x11`

**Text:** Our code takes the i.MX8MP's hwdb row as `gc7000ULN_v123`
(customer 0x11: `src/vivante/profile.rs:107`, `backend.rs:768`), while
`tools/vivante/mesa-cs-golden.patch:84` gives Mesa customer 0, which selects
`gc7000UL_6200` (revision 0x6202; V4Compression 1, REG_ZCompression 1,
Streams 8). No record or test carries the register's value; the passing
board runs only prove revision 0x6204 and product 0x70003. See
`docs/notes/gc-customer-id.md`. Work: (1) a board read of
`VIVS_HI_CHIP_CUSTOMER_ID` (0x30) on the X2 as that note describes (a record
word or two gl_test cases), or the `identity ...: customer 0x..` line from a
Verdin boot; (2) if 0x11: the patch's entry gets 0x11 and the goldens and the
compression statements in `gc-tiled-wire.md`, README and
`gc-slow-*stream-diff.md` are regenerated and corrected (ts-golden);
(3) if 0: `profile.rs`, `backend.rs` and the docs cited in the note move to
the 6202 row, and the tiled design programs compression.
