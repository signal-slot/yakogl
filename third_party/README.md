<!-- SPDX-License-Identifier: Apache-2.0 -->
# third_party: upstream code in its upstream form

This directory holds upstream code in its upstream form, under the rule, the
layout and the update procedure below (`docs/source-form.md`, steps 2 to 4).
Vendored so far: `ti-img-rogue-ddk-25.2/` (the PowerVR DDK headers, step 2),
`mesa-26.2.1/` (the V3D packet generator and tiling, step 4; the PowerVR
PDS generators and csbgen, step 3; the Mali-G610's v10.xml and Mesa's
gen_pack.py, the input of our generated Rust packers and their cross-check;
Arm's Valhall isa-v9-v14.xml and Mesa's valhall ISA.xml and disassembler, the
input of our generated instruction table and its reference text) and `zephyr-pr114866-2026-07-29-2473da6b/`
(TI's Zephyr DSI driver, the data source where the AM62Px TRM is silent;
group E step 7).

## The rule

1. **Code brought in from elsewhere stays in its upstream form.** It is
   copied unmodified from a pinned upstream version, together with its own
   licence files, so that it can be updated by copying a newer version over it.
   Our adaptations live in separate files outside this directory
   (`driver/compat/` for shims). A file an upstream generates is produced by
   that upstream's own generator, either in the build directory or committed
   with a check that regenerating it gives the committed file.
2. **The one exception** is code whose licence does not let it be brought in
   (the GPL-only Linux drivers). It is not vendored and not ported: it is
   replaced by a permissively licensed driver or rewritten from the hardware
   documentation, and such a rewrite is original code outside this directory.
3. **Everything original to the project is Rust.** C is kept only where
   Zephyr or Linux registration forces it, and then kept thin.

A file under `third_party/` is never edited in place. A change we need in an
upstream file is a patch in `third_party/patches/`, with its reason, applied
by the build, and offered upstream.

## Layout

```
third_party/
  README.md                  this file
  UPSTREAM.template          the UPSTREAM file format, to copy
  <upstream>-<version>/      one directory per upstream and pinned version
    UPSTREAM                 where the files came from (format below)
    LICENSE, COPYING, ...    the upstream's own licence files, unchanged
    <the upstream's paths>   the vendored files, at their upstream paths
  patches/
    <upstream>-<version>/NNNN-<what>.patch   local changes, one per reason
```

`<upstream>` is a short lower-case name (`mesa`, `ti-img-rogue-ddk`,
`etna_viv-rnndb`); `<version>` is the upstream's release tag, or the date and
short commit when there is no tag (`mesa-26.2.1`,
`ti-img-rogue-ddk-25.2`). Files keep their upstream relative paths, so an
upstream path cited in a comment is also the path under the directory.

Each vendored directory gets a line in `NOTICE` (licence, copyright holders,
what uses it) and its licence text in `LICENSES/`.

## The UPSTREAM file

Plain `key: value` lines, then the file list. `UPSTREAM.template` is the
blank form.

```
name:      Mesa
url:       https://gitlab.freedesktop.org/mesa/mesa.git
version:   26.2.1
commit:    <full commit id the tag points at>
date:      <date the files were copied, YYYY-MM-DD>
licence:   MIT (SPDX expression of what is vendored)
copyright: Imagination Technologies Ltd.; Broadcom; ...
licence-files: docs/license.rst
generated: <generator command, input, output> (one line each, or "none")
patches:   <patches/mesa-26.2.1/NNNN-*.patch, one per line, or "none">
files:
  src/util/macros.h
  src/imagination/vulkan/pds/pvr_pds.c
  ...
```

- `files` lists every vendored file, one per line, relative to the directory.
  Nothing is in the directory that is not listed, and nothing listed is
  missing.
- `generated` names each file produced from vendored input and how. Its output
  goes to the build directory; when it is committed instead, the build or
  `tools/check.sh` regenerates it and fails when the result differs.
- `patches` lists the local patches in the order they apply, each with the
  reason in its own header.

## Updating an upstream

1. Copy the new upstream version into a new `<upstream>-<newversion>/`
   directory, taking exactly the files the old `UPSTREAM` lists (plus any the
   new version needs), and write its `UPSTREAM` with the new version, commit,
   date and file list. Copy the upstream's licence files unchanged.
2. Re-apply `patches/<upstream>-<oldversion>/` onto the new tree; move each
   patch that still applies to `patches/<upstream>-<newversion>/`, rewrite the
   ones that do not, and drop the ones upstream has absorbed.
3. Regenerate every generated file listed in `generated`.
4. Point the build at the new directory, build every row, and run the checks
   that pin the behaviour (the host harness, the golden and cross-check
   tools the group's entry in `docs/source-form.md` names).
5. Remove the old directory in the same commit, update `NOTICE`, and say in
   the commit which upstream changes reached us.
