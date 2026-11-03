# Nestwatch website

The static site served at <https://nestwatch.appmire.be/>: home, download,
privacy, self-hosting, open source and FAQ, in English (`/`) and Dutch
(`/nl/`).

```sh
apps/site/build.sh                 # → target/nestwatch-site
apps/site/build.sh /some/dir
python3 -m http.server -d target/nestwatch-site 8000
```

Needs Python 3.12+ only. The build fails when `i18n/nl.json` and
`i18n/en.json` differ in structure or when an internal link or anchor does
not resolve.

| Path | Contents |
|------|----------|
| `build.py` | page templates, release asset names (`ASSETS`), sitemap, robots.txt, link check |
| `i18n/<lang>.json` | every visible string, one file per language, same structure |
| `site.css` | page layout on top of `design/nestwatch.css` (copied verbatim) |
| `site.js` | theme toggle and phone menu; no network access, no cookies |
| `img/` | crops of `design/preview-*.png`, Open Graph cards, touch icon |
| `tools/render-images.mjs` | re-renders `img/og-*.png` and `img/icon-180.png` with headless Chrome |

Privacy rules (security.md PRIV-1) apply to the site too: no CDN, no web
fonts, no analytics, no cookies. Screenshots are design previews only, never
real camera images.

To add a language: copy `i18n/en.json`, translate the values (keep `icon`,
`tone`, `image`, `key` and `id` as they are), and add the code to `LANGS` in
`build.py`.
