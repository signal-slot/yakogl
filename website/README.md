<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The YakoGL website

Static HTML, CSS and JavaScript, written by hand: no generator, no build
step, no external resources.

- `index.html`, `status.html`, `try-it.html`, `404.html`: the pages.
- `data.js`: every figure the pages show, with its value, unit, commit,
  date and source record. A new measurement changes this file only;
  `site.js` renders it. The format is described at the top of the file.
- `style.css`: the one stylesheet (light and dark).
- `hero.js`: the home page's WebGL background; the page works without it.
- `og-image.png`: written by `tools/website-og-image.py`.

Check it with `tools/website-check.sh` (relative links, page metadata,
`data.js` against its schema and the repository). Preview it by opening
`index.html` in a browser, or with `python3 -m http.server -d website`.

## Publishing

`.github/workflows/pages.yml` deploys this directory with GitHub Pages on
every push to `main` that touches it, and on demand. It does nothing until
Pages is turned on: when the repository goes public, set Settings > Pages >
Source to "GitHub Actions" and the repository variable `PAGES_ENABLED` to
`true` (Settings > Secrets and variables > Actions > Variables; the job is
skipped without it, because a private repository on the free plan has no
Pages and the deploy would fail on every push). There is no custom domain and no `CNAME`; the
site lives at the address GitHub gives it. Once it is known, put it in front
of the `<loc>` entries of `sitemap.xml` and add a `Sitemap:` line to
`robots.txt`.
