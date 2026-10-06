# `tools/cts/testdata/`

Fixtures for the tools in `tools/cts/`, so a parser can be checked without a
board and without a 40-minute sweep.

`refusals-new-format.qpa` — three `TestCaseResult` blocks in dEQP's own XML,
carrying the online compiler's message format from `fc88b41`:

    0:59: &apos;.&apos; : assignment to this is not lowered yet

It exists because that format nearly cost a whole sweep. `cts-refusals.py`'s
first pattern excluded `&` from the message so it would stop at an entity —
correct while the message had no token, and silently capturing **nothing** the
moment one appeared, because dEQP escapes the quotes around the token as
`&apos;` and they begin immediately after the colon. Every refusal would have
bucketed under the empty string, and the histogram would have been one nameless
row. The fixture pins:

- the entity-escaped token is decoded and split from the message
- a case dEQP marked `Pass` is counted as a **correct** refusal, not a gap
- one message over two different tokens splits by construct

`deqp-config/` — resolved Zephyr configurations in the form a build writes
its `zephyr/.config` (`CONFIG_X=value` lines, a disabled bool as
`# CONFIG_X is not set`), fed to `check-deqp-images.py --config` by
`cts-selftest.sh` check 17. The dEQP image builders run that same check on
every image they build, and the workers have no Zephyr build, so these
fixtures are its only host evidence:

- `am62p-sweep.config`, `imx8mp-sweep.config`, `pi4-sweep.config` and
  `am62p-ms4.config` (with `--ms4`) — one conformance image per board plus
  the multisample one; the check must pass each
- `bad-*.config` — one broken image per rule the check enforces: the
  compiler off (`bad-compiler-missing`), the Mesa table chosen
  (`bad-shaderc-offline`), precompiled-only (`bad-precompiled-only`), the
  precompiled tier in front of the compiler (`bad-precompiled-tier`), ES 3.0
  reporting (`bad-report-es3`), the sample's ES 2.0 switch gone
  (`bad-gles3-report`), a sweep image on the multisample render config
  (`bad-sweep-ms4-name`), and the multisample image without it
  (`bad-ms4-name-missing`); the check must fail each with a message naming
  the offending symbol
