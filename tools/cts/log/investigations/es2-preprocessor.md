<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GLSL ES 1.00 preprocessor: the dEQP-GLES2 `define_value_and_function` and macro-order cases

**Status (2026-10-11): finished, no defect.** The two failures were one stale
row of `tools/cts/RESULTS.md`, and all 534 preprocessor shaders get the CTS's
compile outcome on the host; HANDOFF records this. Not landed: the guard test
of section 7 (`tests/es2_preprocessor.rs` does not exist).

Item `es2-preprocessor-diag` of plan #4. Host only, no source changed. Every claim is marked
**[measured]** (run on the host in this session), **[recorded]** (read from a committed record, not re-run) or
**[inferred]** (reasoning from code or the spec, not run).

## Result in one paragraph

**There is no preprocessor defect left to fix for the dEQP-GLES2 cases.** The two "unowned" failures,
`dEQP-GLES2.functional.shaders.preprocessor.definitions.define_value_and_function_{fragment,vertex}`,
come from one row of `tools/cts/RESULTS.md` (line 7623, "514 / 516 ... unowned", measured at online pair
`e31bd3e`, 2026-09-23). At the integration tip (`2c7f887`) the online preprocessor expands that shader
correctly, and all 534 preprocessor shaders of `preprocessor.test` (the whole file) get the compile outcome
the CTS expects **[measured]**. Later board sweeps record the whole preprocessor group passing on both
PowerVR and V3D **[recorded]**: `indexing, preprocessor 1,100 / 1,100` on the AM62P (`RESULTS.md` 12710), and
`preprocessor 532, 0 failed` in an S3 sweep (`RESULTS.md` 9843, 11714; 9843 is the Pi 4 sweep 28 section). The stale row is the only thing that still lists the two cases as
failing. There is no separate "macro expansion order" failure: the only case about it,
`semantic.correct_order`, passes **[measured]**; the task text appears to have grouped it with the stale row
**[inferred]**.

What is real is a set of preprocessor divergences from C99 that **no dEQP-GLES2 case exercises** (section 4).
They are candidates for `es2-preprocessor` if the owner wants them; none is on the stage-1 critical path.

`pp-splice` does not affect any of these cases (section 5).

## 1. What the two cases are

`data/gles2/shaders/preprocessor.test`, group `definitions`, expected `out0 = 6.0`:

```glsl
precision mediump float;
${DECLARATIONS:single-line}
#    define        VALUE            (1.5 + 2.5)
#    define        FUNCTION(__LINE__, b)    __LINE__+b

void main()
{
    out0 = FUNCTION(VALUE, ((0.2) + 1.8) );
    ${OUTPUT}
}
```

The trap is the parameter named `__LINE__`: the parameter must shadow the predefined macro inside the
replacement list, so the body is `VALUE + b`, not `<line number> + b`. Correct result: `(1.5 + 2.5) + ((0.2) + 1.8)` = 6.0.

The "order" case is `semantic.correct_order` (expected `out0 = 1.0`):

```glsl
#define FUNC(A) A
#define A 2.0
void main() { out0 = FUNC(A - 1.0); }   // argument expanded first: 2.0 - 1.0
```

## 2. Method [measured]

A throwaway test (`tests/zz_pp_scratch.rs`, not committed, deleted before the commit) parsed
`preprocessor.test`, built each `both`/`vertex`/`fragment` source the way
`glsShaderLibraryCase.cpp` does (`${DECLARATIONS}`, `${DECLARATIONS:single-line}`, `${OUTPUT}`,
`${POSITION_FRAG_COLOR}`, `${VERTEX_*}`, `${FRAGMENT_*}`; `out0` as float or vec4 as the case's `values`
say), ran `yakogl::shaderc::online::pp::preprocess` for the token stream and
`SHC_ONLINE.compile` for the verdict, with
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online`.
534 shaders (fragment and vertex of each case; `pragmas.pragma_*` have both stages in one case).
Verdict compared with `expect compile_fail` or not. **0 mismatches** (the first run had 10, all from the
throwaway harness: float `out0` where the case uses vec4, and unsubstituted `${VERTEX_DECLARATIONS}`).

Limit: this checks pass/fail of compilation and, for the cases below, the expanded token stream. It does not
run the shader and compare an image, so a back-end defect that turns a correctly expanded shader into a wrong
picture would not show. The board sweeps in `RESULTS.md` are the evidence for the picture **[recorded]**.

The mustpass list `tools/cts/gles2-mustpass.txt` has 516 `preprocessor.*` names. 20 of the 534 host shaders
are not in it (reason not established): `basic.identifier_with_double_underscore`,
`builtin.line`, `builtin.line_defined_1`, `definitions.undefine_invalid_object_1`, `..._2`,
`extensions.after_non_preprocessing_tokens`, `invalid_redefinitions.invalid_identifier_2`,
`predefined_macros.line_2` (each fragment + vertex) and the four stage shaders of
`pragmas.pragma_vertex` / `pragma_fragment` (the list has them as one case each). They also match.

## 3. Table: related mustpass cases

Fragment and vertex rows are identical for every case **[measured]**. "yakogl" is the host verdict and, for
value cases, the expanded `main` body at the tip. "Cause" is empty when the case is fine.

### 3.1 The reported failures and the same feature

| case | CTS expects | yakogl today [measured] | cause |
|---|---|---|---|
| `definitions.define_value_and_function` | build; `out0 = 6.0` | builds; `out0 = ( 1.5 + 2.5 ) + ( ( 0.2 ) + 1.8 ) ;` = 6.0 | none now. `substitute()` (pp.rs) looks a body identifier up in the parameter list *before* `expand_into` can see `__LINE__`, so the parameter wins. Board: stale row (RESULTS.md 7623); passes in 12710 / 9843 [recorded] |
| `semantic.correct_order` | build; `1.0` | builds; `out0 = 2.0 - 1.0 ;` | none: `substitute()` expands each argument (`expand_into`) before splicing it |
| `semantic.ops_as_arguments` | build; `20.0` | builds; `float ( ( 1 + 9 ) * 2 )` | none |
| `function_definitions.same_object_and_function_param` | build; `1.0` | builds; `( 3.0 - 2.0 )` | none: parameter `VALUE` shadows object macro `VALUE` |
| `function_definitions.complex_func` | build; `518.5` | builds; `8.0 / 4.0 * ( 8.0 / 4.0 - 2.0 ) * 2.0 * 2.0 - 0.75 * 2.0 - 40.0 * ( 40.0 - 10.0 * 5.0 - 3.0 )` = 518.5 | none (nested and inter-calling macros) |
| `function_definitions.function_definition_with_comments` | build; `3.0` | builds; `1.0 + 2.0` | none |
| `definitions.undefine_function` | build; `1.0` | builds; `3.0 - 2.0` | none |

### 3.2 Recursion (expected compile failure; all fail as expected)

| case | CTS | yakogl [measured] |
|---|---|---|
| `recursion.recursion_1` `#define AAA AAA` | fail | fail: `recursive macro definition` |
| `recursion.recursion_2` `AAA`/`BBB` cross | fail | fail, same message |
| `recursion.recursion_3` `(1.0+BBB)`/`(2.0+AAA)` | fail | fail, same message |
| `recursion.recursion_4` `AAA(a) AAA(a)` | fail | fail, same message |
| `recursion.recursion_5` `AAA(a, b) AAA(b, a)` | fail | fail, same message |

Mechanism: `expand_into()` keeps `active_macros`; meeting an active name is an error, not C's "blue paint"
(deliberate, module note at the top of pp.rs).

### 3.3 `#undef`, redefinition, predefined names

| case | CTS | yakogl [measured] |
|---|---|---|
| `definitions.undefine_object_invalid_syntax` | fail | fail: extra tokens after `#undef VAL` |
| `definitions.undefine_invalid_object_1..4` (`__LINE__`, `__FILE__`, `__VERSION__`, `GL_ES`) | fail | fail: `cannot undefine a predefined macro` |
| `invalid_definitions.define_non_identifier`, `undef_non_identifier_1/2` | fail | fail |
| `function_redefinitions.function_redefinition_1..3` | build | builds (identical redefinition accepted) |
| `function_redefinitions.invalid_function_redefinition_param_1..3` | fail | fail: `macro redefined with a different definition` |
| `object_redefinitions.*` (15 cases: 7 `invalid_object_*` fail, 8 `redefine_object_*` build) | as named | all as expected |
| `invalid_redefinitions.invalid_identifier_2` (`GL_` prefix) | fail | fail |
| `invalid_function_definitions.*` (16: arguments_1..8, unique_param_name, argument_list_1..3, no_closing_parenthesis_1..4) | fail | all fail (pp error, or a later parse/sema error for arguments_1, _4, _6, _7) |
| `basic.empty_object/empty_function/empty_directive/null_directive/identifier_with_double_underscore` | build | build |
| `basic.invalid_directive`, `missing_identifier`, `invalid_identifier`, `correct_phases` | fail | fail |
| `comments.*` (7) | as named | all as expected |

### 3.4 `__LINE__`, `__FILE__`, `#line`

| case | CTS | yakogl [measured] |
|---|---|---|
| `builtin.line` (`#line 1`, `__LINE__`) | 1.0 | `__LINE__` -> 1 |
| `builtin.line_and_file` (`#line 234 10`) | 234,234,10,10 | 234,234,10,10 |
| `builtin.line_defined_1` (`#define A 4`, `#line A`) | 4.0 | 4 |
| `builtin.line_defined_2` (`#line 234 A`, A=10) | 234,234,10,10 | 234,234,10,10 |
| `builtin.empty_line`, `invalid_line_file_1/3/4` | fail | fail (`#line requires ...`, extra tokens) |
| `predefined_macros.line_1` (`const int line = __LINE__;`) | 1.0 | line 1 |
| `predefined_macros.line_2` (`__LINE__` in a body, an argument, over several lines and a comment) | `11, 11, 9, 10` | `__LINE__` -> 11, 11, 9, 10 (invocation line is where it ends; arguments keep their own line) |
| `predefined_macros.file`, `version`, `gl_es_1`, `gl_es_2`, `if_gl_es`, `if_version` | 0.0, 100.0, 1.0 ... | `__FILE__` 0, `__VERSION__` 100, `GL_ES` 1; all build |

Mechanism: `predefined_value()` and the `invocation_line` stack in `expand_into()`.

### 3.5 Macros that expand to `defined`, `#if`, `#pragma`, `#extension`

| case | CTS | yakogl [measured] |
|---|---|---|
| `conditional_inclusion.basic_1..5`, `unary_ops_1/2` (`basic_2`: `#define AAA defined(BBB)`, `#if !AAA`) | build; 1.0 | build; 1.0 |
| `invalid_ops.invalid_op_1/2`, `invalid_defined_expected_*` (6) | fail | fail |
| `invalid_ops.defined_define` (`#define define 1`) | build | build |
| `expressions.defined_1..6` | build; 1.0 | build; 1.0 |
| `pragmas.pragma_macro_exp`, `extensions.macro_exp` | build | build |

All other groups of `preprocessor.test` (conditionals, invalid_conditionals, directive, expressions,
invalid_expressions, operator_precedence, undefined_identifiers, extensions, pragmas) also match:
per group, every shader matched **[measured]**.

## 4. Divergences found that the CTS does not ask for [measured, extra shaders]

Written as `precision mediump float;` + defines + `void main() { gl_FragColor = vec4(EXPR); }`.
"Expected" is C99 / GLSL ES 3.00 preprocessing, as I read them **[inferred]**; ES 1.00 section 3.4 is
silent or terse on these points. None is in dEQP-GLES2 (`grep` of `preprocessor.test` for `##` and `#x`: none).

| id | shader (defines; EXPR) | expected [inferred] | yakogl [measured] | cause [inferred from code] |
|---|---|---|---|---|
| E2 | `#define X(a) a`, `#define Y X`; `Y(2.0)` | `2.0` | `X ( 2.0 )` left unexpanded; later `no function of this name is declared` | `expand_into()` expands a macro body on its own: a function-like name that ends the replacement list has no `(` in `toks`, so it falls into "name not followed by `(`" and is emitted. C rescans the replacement list *together with the rest of the source* |
| E4 | `#define S(x) #x`; `1.0` | ES 1.00: invalid (no stringize) | accepted | `d_define()` takes any tokens as the body; `#` in a body is never diagnosed unless the macro is used |
| E5 | `#define C(a,b) a##b`; `1.0` | ES 1.00: invalid | accepted | same |
| E10 | `#define F(x) x`; `F(F)(1.0)` | C: `F(1.0)` unexpanded -> error later | `recursive macro definition` | same outcome class (fails), different message |
| E12 | `#define X 1`, `#define D defined(X)`; `#if D` | C: undefined behaviour; common compilers evaluate `defined(X)` | `expected an identifier after 'defined('` | `eval_condition()` expands `D` (so X -> 1) and only then folds `defined` again; the existing comment in pp.rs says this limit is known. dEQP `basic_2` passes because its operand is undefined |

Matching C: E1 (object then function redefinition fails), E3 (`A(A(1.0))`), E6 (`#undef` of an unknown
name), E7 (indirect recursion fails), E8 (`H()`), E11 (`__LINE__` in argument and body), E13 (parameter named
`__LINE__`), E14 (use after `#undef`).

## 5. Pending work

**[measured]** `git log --all -- src/shaderc/online/pp.rs src/shaderc/online/lexer.rs` shows one commit (`afb1633`,
the squashed import); no ref of the agents' result branches has a name containing `splice`, `pp-` or
`preproc`, and no ref's history touches `pp.rs` separately. The plan says `pp-splice` is "in review" in plan #3 and
its branch was not in this clone, so its diff was not read.

**[measured]** the line-continuation splice already in the tree is ES 3.00 only:
`lexer.rs` `Version::line_continuation()` gates `splice_lines()` (line 531) and the `\` arm of the tokenizer
(line 644); `tests/es3_shaderc_pp.rs` pins it and says "GLSL ES 1.00 has no line continuation and keeps
refusing the same splice". **[inferred]** `pp-splice` is the same line-splice work for ES 3.00. All cases in
section 3 are `#version 100` (no `#version` line, or `#version 100`) and contain no backslash-newline, so a
splice change cannot alter their tokens. It does not affect E2, E4, E5, E10 or E12 either: none involves a
continuation. The one place the two meet is `__LINE__`/`#line` accounting after a splice, which for ES 3.00 only.

## 6. Proposal for `es2-preprocessor`

1. **Close the two "failures" by evidence, not by code.** Update `tools/cts/RESULTS.md` line 7623's
   "unowned" note (or its successor in `docs/es2-cts-failures.md`, which does not list them) as stale, and
   cite the passing sweeps. Owner of the doc: `es2-docs-r1` / `es2-docs-final`.
2. **Add `tests/es2_preprocessor.rs`** as a regression guard (cases in section 7), gated like
   `tests/es3_shaderc_pp.rs` but for the ES 2.0 reporting build too (`shaderc-online`, `gpu-powervr`), so the
   next change to `substitute()` or `predefined_value()` cannot reintroduce the `__LINE__`-parameter bug.
   No change to pp.rs or lexer.rs is needed to make it pass.
3. Optional hardening, only if the owner wants C99-faithful behaviour (nothing in dEQP-GLES2 requires it):
   - E2: in `expand_into()`, when a function-like macro name is the last token of a replacement list, look
     past the end of `toks` into the caller's remaining tokens (the usual fix: expand over a token stream with a
     pushback, or let `expand_into` take the unconsumed tail of the outer list). Touches `expand_into`,
     `collect_args`; careful with `active_macros` (the name must stay hidden while its body is rescanned).
   - E4/E5: reject `#` and `##` tokens in `d_define()` bodies when `version == 100`, if the lexer yields them as
     distinct `Tok` kinds (`Tok::Unknown` or the directive marker; check `lexer.rs`), error text
     "'#' / '##' is not supported in GLSL ES 1.00". Risk: the spec text only forbids them, glslang accepts
     `#` as an error only at use. Decide with the owner; dEQP has no case.
   - E12: in `eval_condition()` fold `defined` operands that came from a macro body before expanding the
     operand (mark the tokens that came from a replacement list). Out of scope for ES 2 CTS.

## 7. What `tests/es2_preprocessor.rs` should contain

Each entry: shader text (`<prelude>` = `precision mediump float;\n`, body in `void main() { ... }`), expected outcome.
Check by compiling through `SHC_ONLINE.compile` for fragment, and by `pp::preprocess` token text for the value
cases. Outcomes below were all **[measured]** at the tip with the harness above.

| # | shader text | expected |
|---|---|---|
| 1 | `#define VALUE (1.5 + 2.5)\n#define FUNCTION(__LINE__, b) __LINE__+b` ; `out = FUNCTION(VALUE, ((0.2) + 1.8) );` | builds; tokens `( 1.5 + 2.5 ) + ( ( 0.2 ) + 1.8 )` |
| 2 | `#define FUNC(A) A\n#define A 2.0` ; `FUNC(A - 1.0)` | builds; `2.0 - 1.0` |
| 3 | `#define FOO(a, b) (1 a 9) b 2` ; `float(FOO(+, *))` | builds; `( 1 + 9 ) * 2` |
| 4 | `#define VALUE 1.0\n#define FUNCTION(VALUE, B) (VALUE-B)` ; `FUNCTION(3.0, 2.0)` | builds; `( 3.0 - 2.0 )` |
| 5 | `#define AAA(a,b) a*(BBB(a,b))\n#define BBB(a,b) a-b` ; `BBB(AAA(8.0/4.0, 2.0)*BBB(2.0*2.0,0.75*2.0), AAA(40.0,10.0*BBB(5.0,3.0)))` | builds; value 518.5 |
| 6 | `#define F(a,b) a+b\n#undef F\n#define F(a,b) a-b` ; `F(3.0, 2.0)` | builds; `3.0 - 2.0` |
| 7 | `# define AAA AAA` ; `AAA` | compile failure |
| 8 | `# define AAA BBB\n#define BBB AAA` ; `AAA` | compile failure |
| 9 | `# define AAA (1.0+BBB)\n#define BBB (2.0+AAA)` ; `AAA` | compile failure |
| 10 | `# define AAA(a) AAA(a)` ; `AAA(1.0)` | compile failure |
| 11 | `# define AAA(a, b) AAA(b, a)` ; `AAA(1.0, 2.0)` | compile failure |
| 12 | `#undef __LINE__`, likewise `__FILE__`, `__VERSION__`, `GL_ES` | compile failure each |
| 13 | `#define BBB __LINE__, /*\n */ __LINE__\n#define AAA(a,b) BBB, a, b` with `AAA(__LINE__,\n __LINE__\n )` on three lines (the CTS `predefined_macros.line_2` layout) | builds; `__LINE__` values 11, 11, 9, 10 when the shader is laid out as the CTS does (`precision` on line 1) |
| 14 | `#line 234 10` then `vec4(__LINE__, __LINE__, __FILE__, __FILE__)` | values 234, 234, 10, 10 |
| 15 | `#define A 4\n#line A` then `__LINE__` | 4 |
| 16 | `#define AAA defined(BBB)\n#if !AAA ... #else ... #endif` with BBB undefined | takes the `#if` branch |
| 17 | `#define F(x) x\n#define F 1.0` | compile failure (redefinition differs) |
| 18 | `#define H() 1.0` ; `H()` | builds; `1.0` |
| 19 | `#define F(x) G(x)\n#define G(x) F(x)` ; `F(1.0)` | compile failure |

Add as `#[ignore]`d or "known gap" tests only if `es2-preprocessor` takes the optional hardening of section 6:
E2 `#define X(a) a\n#define Y X` ; `Y(2.0)` should build with `2.0`; E4/E5 should fail on `#`/`##` in a body.
The tip does the opposite today (measured, section 4).

## 8. Facts for whoever picks this up

- Scratch outputs are not kept; to reproduce, parse `preprocessor.test` as in section 2.
- Everything here is on the PowerVR host build; the preprocessor is shared by all back ends (`pp.rs` is
  back-end independent **[inferred]**), but the GC7000 and V3D builds were not re-run.
- `tools/cts/RESULTS.md` line 7623 should be read as superseded; I did not edit it (outside the item).
