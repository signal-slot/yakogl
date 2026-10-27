<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The ES 1.00 twins of gl_test's GLSL 450 shaders

`driver/tools/pvrlab/gl_test_shaders.h` holds gl_test's shaders, and fourteen
of them are `#version 450` written for SPIR-V: `layout` locations, Vulkan
`push_constant` blocks, descriptor-set samplers, a user fragment output. An
ES 2.0 application cannot write any of that, and the in-process compiler is an
ES 2.0 compiler, so gl_test refused at line 1 through the online path — case 0
and case 1 included.

These are the same fourteen shaders as an ES 2.0 application would write them:
`attribute` / `varying` / `uniform`, precision qualifiers, `gl_FragColor`,
`texture2D`. They are **twins, not replacements**. `gl_test.c` takes this set
in every build that compiles its shaders: when `GL_TEST_ES100` is defined,
or in a Zephyr image with the in-process compiler
(`CONFIG_YAKOGL_SHADERC_ONLINE`) or with the precompiled shaders
(`CONFIG_YAKOGL_PRECOMPILED_SHADERS`). Only a build without either takes
the 450 set, through the Mesa reference table.

Two rules make the set an oracle rather than a rewrite, and both are checked
rather than intended:

* every name is the same as its twin's — `a_pos`, `a_color`, `a_uv`, `a_size`,
  `a_z`, `v_color`, `v_uv`, `u_color`, `u_mvp`, `u_tex`, `u_tex0`, `u_tex1` —
  so the reflection each produces (attributes, uniforms, varyings, field by
  field) must match, and a test says so;
* the declaration ORDER is the twin's location order, because ES 1.00 has no
  `layout(location=)` and an attribute's location comes from the binding the
  application makes or the order the compiler assigns. Reordering a
  declaration here would silently move an attribute.

The behaviour must be identical too: the same case, drawing the same geometry,
must read back the same pixels with either set. That is what proves the
rewrite preserved the tests before the online compiler is judged by them.
