Fixtures of tools/cts/am62p-gles3-e2e-selftest.sh: redacted, fake console and note
snippets (no board address, UART path, host name or home path). The script fills in
@CASE@, @ROW@, @SHA8@ and @ROWS@ and builds the rest (record, tables, fake mustpass
list) under $TMPDIR.
- report-expected.md: the expected output of am62p-gles3-report.py on the fake records (build sha8 as @SHA8@).
