# Mali dEQP-GLES2 failure triage (draft)

Slices: `2026-12-01-imx95-gles2-S1-e63ab28e`, `2026-12-01-imx95-gles2-S2-e63ab28e`.
Forecast: `docs/notes/mali-gles2-forecast.md`.

| Cause | Cases | Examples | Slices | Forecast | Owner |
| --- | ---: | --- | --- | --- | --- |
| hang: WDOG3 feeding stopped | 2 | `dEQP-GLES2.functional.b.one`, `dEQP-GLES2.functional.b.two` | `2026-12-01-imx95-gles2-S1-e63ab28e` | yes | none (not a Fail: the row hung) |
| hang: lifetime over | 1 | `dEQP-GLES2.functional.c.one` | `2026-12-01-imx95-gles2-S1-e63ab28e` | yes | none (not a Fail: the row hung) |
| refused ARITH | 1 | `dEQP-GLES2.functional.a.two` | `2026-12-01-imx95-gles2-S1-e63ab28e` | yes | plan #309 |
| unexplained Fail in functional.d | 1 | `dEQP-GLES2.functional.d.one` | `2026-12-01-imx95-gles2-S2-e63ab28e` | **NEW** | - |

Non-Pass cases: 5.

## Draft issues

### Draft issue: Mali dEQP-GLES2: unexplained Fail in functional.d (1 case)

Title: Mali dEQP-GLES2: unexplained Fail in functional.d (1 case)

Body:

- Cause: unexplained Fail in functional.d
- Cases: 1 (Fail 1)
- Examples: `dEQP-GLES2.functional.d.one`
- Slices: `2026-12-01-imx95-gles2-S2-e63ab28e`
- Commits: `e63ab28e`
- Kernels: zephyr verdin_imx95
- Detail (numbers and names stripped): `Fail` x1
- How found: A non-Pass with no refusal message, outside the forecast's causes.
- Forecast: not named in `docs/notes/mali-gles2-forecast.md`; add it to section 2 when the issue is open.
