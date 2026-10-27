<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The YakoGL website

Static HTML, CSS and JavaScript. No generator, build step, external fonts or
external scripts. The canonical public origin is `https://yako.gl`.

- `index.html`: what runs, board selection and the next step.
- `how-it-works.html`: architecture and shader compilation choices.
- `status.html`: measured results, sources and current limitations.
- `try-it.html`, `try-it.js`: the real-hardware board guide; `?board=am62p`,
  `?board=imx8mp` or `?board=pi4` selects a target. Without JavaScript all
  instructions remain available. Host-side development tests stay in the repository
  documentation rather than being presented as a YakoGL demo.
- `404.html`: the not-found page.
- `data.js`: measurement values and their provenance, rendered by `site.js`.
  Its header describes the schema. Pre-publication builds retain their
  identifiers and link to the record in the first public snapshot; they do
  not link to unavailable development commits or issue numbers.
- `style.css`: the shared stylesheet, responsive in light and dark schemes.
- `og-image.png`: written by `tools/website-og-image.py`.

The home page uses a labelled stack diagram. It does not present browser
WebGL or an illustrative UI as a capture of YakoGL running on a board.
The older `hero.js` and `home.js` effects are not loaded by the pages.

Check with `tools/website-check.sh` (links, metadata and measurement
provenance). Before a pull request, run `tools/check.sh --changed origin/main`
as described in `CONTRIBUTING.md`. Preview with `python3 -m http.server -d website`.

## Publishing

`.github/workflows/pages.yml` uploads `website/` to GitHub Pages on pushes
to `main` that touch the site, and on demand. It runs only when the
repository variable `PAGES_ENABLED` is `true` and Pages is configured to
use GitHub Actions. DNS and the hosting account's custom-domain settings
are managed separately; there is no `CNAME` in this directory.

Canonical URLs, social preview images, `robots.txt` and `sitemap.xml` use
`https://yako.gl`. If the public origin changes, update them together with
`SITE_ORIGIN` in `tools/website-check.sh`. New pages belong in the sitemap;
error pages do not.
