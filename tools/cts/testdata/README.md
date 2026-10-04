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
