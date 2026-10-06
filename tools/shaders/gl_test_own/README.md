<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test's own shaders: cases 181 to 183 and 188 to 192

The GLSL ES 1.00 sources that gl_test cases 181 `fusemad`, 182 `fsat`, 183
`unipad`, 188 `predif`, 189 `mulone`, 190 `blendonce`, 191 `varyfma` and 192
`predlive` draw with. They were written in `gl_test.c` itself, as C string
literals (some assembled by macros), which no shader list could name: an image
without the run-time compiler refused the eight cases by name, because the
precompiled table had no rows for sources it had never seen.

They are files here so that `gen_gl_test_shaders.py` puts them in both gl_test
headers (`gl_test_<name>_vs_src` / `_fs_src`, byte-identical to these files),
`tools/precompile/gl_test_pairs.py` reads the programs the cases link out of
`gl_test.c`, and `tools/shaders/precompiled.list` lists them for the
precompiled table. The host tests of the same cases (`tests/usc_limits.rs`)
compile these files too, so the board and the host see one text.

The directory has no table role (`dir gl_test_own -` in the list): neither
Mesa reference table holds these sources. They are ES 1.00 already, so both
the `gl_test` and the `gl_test_es100` apps link the same files; there are no
twins.

Several cases share `fma_plain.vert` (position only) and `fma_varying.frag`
(the varying `v` as the colour): a case's fragment-shader shape runs under
`fma_plain.vert`, and its vertex-shader twin (`*_varying.vert`) feeds
`fma_varying.frag`.
