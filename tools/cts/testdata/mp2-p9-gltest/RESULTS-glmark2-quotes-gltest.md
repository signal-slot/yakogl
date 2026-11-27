Fixture (not real results): the i.MX8MP row's glmark2 cell quotes a gl_test
re-run ("4 of 4 run, case 5 skipped") before the gl_test cell, which alone
names the nondeterministic cases.

| Board | glmark2-es2 | gl_test | GPU resets |
|---|---|---|---:|
| AM62P | 489; gl_test 4 of 4 run, case 5 skipped | 4 of 4 run, case 5 skipped | 0 |
| i.MX8MP | 42, re-run: gl_test 4 of 4 run, case 5 skipped, none failing | 4 of 4 run, case 5 skipped (#4); cases 2 and 3 nondeterministic (#5) | 0 |
