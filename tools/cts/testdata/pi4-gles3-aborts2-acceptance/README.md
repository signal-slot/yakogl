<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Fixtures for tools/cts/pi4-gles3-aborts2-acceptance-selftest.sh

`root/` is a synthetic repository holding one passing aborts2 record: a
two-image `pi4-gles3-tree.txt` over a four-case mustpass (its header names
`2 images`), and one session directory
`tools/cts/log/2026-11-pi4-gles3-1234abcd-aborts2/` with `README.md`,
`table.txt`, `cycles.txt` and both images' console and tally files. The
table.txt title line names both labels (`A1 B2`), as a real aborts2 record
must (a range names only its ends).

The selftest copies `root/` to a scratch directory, turns it into a git
repository of its own, replaces the README's placeholder `commit:` sha with
the repository's first commit and points `refs/remotes/origin/main` at a
descendant of it, so the ancestor check runs for real; the failing variants
then break exactly one criterion each. Nothing here is executed; the
consoles and tallies only need to exist.

The selftest also renames the session directory to the sha8 of the commit it
was made at (a commit after the repository's first one), and passes the first
commit's sha8 as `PI4_ABORTS2_BASE`, the stand-in for b1b5b845, so c7's
ancestry checks (base before session, session on origin/main) run for real.
