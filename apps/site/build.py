#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Generates the static Nestwatch website (apps/site/build.sh calls this).

All visible text lives in i18n/<lang>.json, one file per language with the
same structure; this script refuses to build when the files drift apart.
Standard library only, so it runs on any CI runner without installing
anything.
"""

from __future__ import annotations

import html
import json
from html.parser import HTMLParser
import shutil
import sys
from datetime import date
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BASE_URL = "https://nestwatch.appmire.be"
REPO = "https://github.com/jeroentrappers/nestwatch"
LATEST = f"{REPO}/releases/latest"
DOWNLOAD = f"{LATEST}/download"
LANGS = ["en", "nl"]  # first is the default, served at /
PAGES = ["home", "download", "privacy", "self-hosting", "about", "faq"]

# Release asset names. They carry no version so that
# releases/latest/download/<name> always points at the newest build; keep in
# sync with .github/workflows/release.yml.
ASSETS = {
    "linux": [
        ("deb", "x86_64", "nestwatch-linux-amd64.deb"),
        ("deb", "aarch64", "nestwatch-linux-arm64.deb"),
        ("rpm", "x86_64", "nestwatch-linux-x86_64.rpm"),
        ("rpm", "aarch64", "nestwatch-linux-aarch64.rpm"),
        ("tar.gz", "x86_64", "nestwatch-linux-x86_64.tar.gz"),
        ("tar.gz", "aarch64", "nestwatch-linux-aarch64.tar.gz"),
    ],
    "macos": [("dmg", "Apple silicon + Intel", "nestwatch-macos-universal.dmg")],
    "windows": [("msi", "x86_64", "nestwatch-windows-x86_64.msi")],
    "android": [("apk", "arm64, armv7, x86_64", "nestwatch-android.apk")],
    "web": [("tar.gz", "WebAssembly", "nestwatch-web.tar.gz")],
    "server": [("binary", "Linux x86_64", "nestwatch-server-linux-x86_64")],
    "sums": [("SHA256SUMS", "SHA-256", "SHA256SUMS")],
}

e = html.escape


def page_path(lang: str, page: str) -> str:
    """Site-absolute URL path of a page, e.g. /nl/download/."""
    prefix = "/" if lang == LANGS[0] else f"/{lang}/"
    return prefix if page == "home" else f"{prefix}{page}/"


# ---------------------------------------------------------------- checks


# Keys whose values are identifiers, not text: equal in every language.
SAME_IN_ALL = {"icon", "tone", "image", "key", "id"}


def shape(v, where="$"):
    """Structure of a strings file: keys, list lengths and block types."""
    if isinstance(v, dict):
        return {
            k: (x if k in SAME_IN_ALL else shape(x, f"{where}.{k}")) for k, x in v.items() if k != "_comment"
        }
    if isinstance(v, list):
        return [shape(x, f"{where}[{i}]") for i, x in enumerate(v)]
    return type(v).__name__


def diff_shape(a, b, where="$"):
    if type(a) is not type(b):
        return [f"{where}: {type(a).__name__} vs {type(b).__name__}"]
    if isinstance(a, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{where}.{k}: only in {'first' if k in a else 'second'}")
            else:
                out += diff_shape(a[k], b[k], f"{where}.{k}")
        return out
    if isinstance(a, list):
        if len(a) != len(b):
            return [f"{where}: {len(a)} vs {len(b)} items"]
        return [x for i in range(len(a)) for x in diff_shape(a[i], b[i], f"{where}[{i}]")]
    return [] if a == b else [f"{where}: {a} vs {b}"]


# ---------------------------------------------------------------- pieces


def logo(size=32):
    return f'<img src="/logo.svg" width="{size}" height="{size}" alt="">'


def shot(name: str, alt: str, w: int, h: int, cls="", eager=False) -> str:
    """A design preview in both themes; CSS shows the one matching the theme."""
    load = "eager" if eager else "lazy"
    return (
        f'<span class="shot {cls}">'
        f'<img class="night" src="/img/{name}-night.png" width="{w}" height="{h}" alt="{e(alt)}" loading="{load}" decoding="async">'
        f'<img class="day" src="/img/{name}-day.png" width="{w}" height="{h}" alt="{e(alt)}" loading="lazy" decoding="async">'
        "</span>"
    )


SHOT_SIZES = {
    "dashboard": (1188, 892),
    "station": (404, 834),
    "pairing": (404, 776),
    "add-station": (404, 776),
    "listener-phone": (404, 776),
}


def _svg(body):
    return (
        '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" '
        f'stroke-linecap="round" stroke-linejoin="round">{body}</svg>'
    )


ICONS = {
    "home": _svg('<path d="M3 11l9-7 9 7"/><path d="M5 10v10h14V10"/><path d="M10 20v-5h4v5"/>'),
    "lock": _svg('<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>'),
    "eye-off": _svg('<path d="M3 3l18 18"/><path d="M10.6 6.1A9.7 9.7 0 0 1 12 6c5 0 8.5 4 9.5 6a13 13 0 0 1-2.6 3.3M6.6 7.6A13.4 13.4 0 0 0 2.5 12c1 2 4.5 6 9.5 6 1.6 0 3-.4 4.3-1"/><path d="M9.9 10a3 3 0 0 0 4.1 4.1"/>'),
    "wave": _svg('<path d="M3 12h2M7 8v8M11 5v14M15 8v8M19 10v4M21 12h0"/>'),
    "talk": _svg('<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>'),
    "camera": _svg('<rect x="3" y="7" width="13" height="10" rx="2"/><path d="M16 11l5-3v8l-5-3"/>'),
    "grid": _svg('<rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/>'),
    "bell": _svg('<path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z"/><path d="M10 20a2 2 0 0 0 4 0"/>'),
    "moon": _svg('<path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z"/>'),
    "wifi": _svg('<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.5 16a5 5 0 0 1 7 0"/><circle cx="12" cy="19.5" r=".8" fill="currentColor"/>'),
    "shield": _svg('<path d="M12 3l8 3v6c0 4.5-3.4 8-8 9-4.6-1-8-4.5-8-9V6z"/><path d="M9 12l2 2 4-4"/>'),
}


def paras(items):
    return "".join(f"<p>{p}</p>" for p in items)


def pill(text, kind="plain"):
    return f'<span class="pill {kind}">{text}</span>' if text else ""


def section(id_, heading, body, intro=None, cls=""):
    intro_html = f'<p class="lead muted">{intro}</p>' if intro else ""
    return (
        f'<section class="band {cls}" id="{id_}" aria-labelledby="{id_}-h">'
        f'<div class="page"><div class="band-head"><h2 id="{id_}-h">{heading}</h2>{intro_html}</div>{body}</div></section>'
    )


def cards(items, cols="cards"):
    out = []
    for it in items:
        tag = pill(it.get("pill"), it.get("pill_kind", "plain"))
        icon = f'<span class="icon-dot {it.get("tone", "honey")}" aria-hidden="true">{ICONS[it["icon"]]}</span>' if it.get("icon") else ""
        body = paras(it["body"]) if isinstance(it["body"], list) else f"<p>{it['body']}</p>"
        out.append(
            f'<article class="card feature">{icon}<div class="row-between wrap"><h3>{it["title"]}</h3>{tag}</div>'
            f'<div class="prose muted">{body}</div></article>'
        )
    return f'<div class="{cols}">{"".join(out)}</div>'


def code(text):
    return f'<pre class="code"><code>{e(text)}</code></pre>'


# ---------------------------------------------------------------- layout


def layout(s: dict, lang: str, page: str, body: str) -> str:
    ui = s["ui"]
    p = s["pages"][page]
    other = [x for x in LANGS if x != lang]
    url = BASE_URL + page_path(lang, page)
    alternates = "".join(
        f'<link rel="alternate" hreflang="{x}" href="{BASE_URL}{page_path(x, page)}">' for x in LANGS
    ) + f'<link rel="alternate" hreflang="x-default" href="{BASE_URL}{page_path(LANGS[0], page)}">'
    current = ' aria-current="page"'
    nav = "".join(
        f'<li><a href="{page_path(lang, k)}"{current if k == page else ""}>{ui["nav"][k]}</a></li>'
        for k in PAGES
        if k != "home"
    )
    langs = "".join(
        f'<a class="btn ghost lang" href="{page_path(x, page)}" hreflang="{x}" lang="{x}" '
        f'aria-label="{e(STRINGS[x]["ui"]["lang_aria"])}">{STRINGS[x]["ui"]["lang_short"]}</a>'
        for x in other
    )
    title = s["ui"]["site_name"] if page == "home" else f'{p["title"]} · {s["ui"]["site_name"]}'
    og_title = p.get("og_title", p["title"])
    footer_nav = "".join(f'<li><a href="{page_path(lang, k)}">{ui["nav"][k]}</a></li>' for k in PAGES)
    return f"""<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{e(title)}</title>
<meta name="description" content="{e(p["description"])}">
<link rel="canonical" href="{url}">
{alternates}
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/img/icon-180.png">
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="#0f1017" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#faf6ef" media="(prefers-color-scheme: light)">
<meta property="og:type" content="website">
<meta property="og:site_name" content="Nestwatch">
<meta property="og:title" content="{e(og_title)}">
<meta property="og:description" content="{e(p["description"])}">
<meta property="og:url" content="{url}">
<meta property="og:locale" content="{s["meta"]["og_locale"]}">
{"".join(f'<meta property="og:locale:alternate" content="{STRINGS[x]["meta"]["og_locale"]}">' for x in other)}
<meta property="og:image" content="{BASE_URL}/img/og-{lang}.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="{e(ui["og_alt"])}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{e(og_title)}">
<meta name="twitter:description" content="{e(p["description"])}">
<meta name="twitter:image" content="{BASE_URL}/img/og-{lang}.png">
<meta name="referrer" content="no-referrer">
<link rel="stylesheet" href="/nestwatch.css">
<link rel="stylesheet" href="/site.css">
<script src="/site.js"></script>
</head>
<body class="page-{page}">
<a class="skip btn primary" href="#main">{ui["skip"]}</a>
<header class="site-head">
  <div class="page head-row">
    <a class="brand" href="{page_path(lang, "home")}">{logo()}<span>Nestwatch</span></a>
    <nav class="site-nav" aria-label="{e(ui["nav_label"])}" id="site-nav">
      <ul>{nav}</ul>
    </nav>
    <div class="row head-tools">
      {langs}
      <button class="btn icon ghost theme-toggle" type="button"
        data-label-night="{e(ui["theme_to_day"])}" data-label-day="{e(ui["theme_to_night"])}"
        aria-label="{e(ui["theme_to_day"])}" title="{e(ui["theme_to_day"])}">
        <svg class="moon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z" fill="currentColor"/></svg>
        <svg class="sun" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><circle cx="12" cy="12" r="4.5" fill="currentColor"/><g stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M4.9 19.1l1.8-1.8M17.3 6.7l1.8-1.8"/></g></svg>
      </button>
      <button class="btn icon ghost menu-toggle" type="button" aria-expanded="false" aria-controls="site-nav" aria-label="{e(ui["menu"])}">
        <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M4 7h16M4 12h16M4 17h16" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>
      </button>
    </div>
  </div>
</header>
<main id="main" tabindex="-1">
{body}
</main>
<footer class="site-foot">
  <div class="page stack">
    <p class="disclaimer card" role="note"><strong>{ui["disclaimer_title"]}</strong> {ui["disclaimer"]}</p>
    <div class="foot-grid">
      <div class="stack-s">
        <span class="brand">{logo(24)}<span>Nestwatch</span></span>
        <p class="muted small">{ui["footer_tagline"]}</p>
      </div>
      <nav aria-label="{e(ui["footer_nav_label"])}"><ul class="foot-links">{footer_nav}</ul></nav>
      <div class="stack-s small muted">
        <p>{ui["footer_license"]}</p>
        <p><a href="{REPO}">{ui["footer_source"]}</a></p>
        <p>{ui["footer_privacy"]}</p>
      </div>
    </div>
  </div>
</footer>
</body>
</html>
"""


# ---------------------------------------------------------------- pages


def home(s, lang):
    p = s["pages"]["home"]
    h = p["hero"]
    hero = f"""<section class="hero" aria-labelledby="hero-h">
  <div class="page hero-grid">
    <div class="stack hero-text">
      <div class="row wrap">{"".join(pill(t, k) for t, k in zip(h["pills"], ["calm", "voice", "cam"]))}</div>
      <h1 id="hero-h">{h["title"]}</h1>
      <p class="lead">{h["lead"]}</p>
      <div class="row wrap">
        <a class="btn primary big" href="{page_path(lang, "download")}">{h["cta_download"]}</a>
        <a class="btn ghost big" href="#how">{h["cta_how"]}</a>
      </div>
      <p class="caption-strong">{h["note"]}</p>
    </div>
    <figure class="hero-shot">
      {shot("dashboard", h["shot_alt"], *SHOT_SIZES["dashboard"], eager=True)}
      <figcaption class="caption-strong">{h["shot_caption"]}</figcaption>
    </figure>
  </div>
</section>"""
    pr = p["promise"]
    promise = section("promise", pr["heading"], cards(pr["items"], "cards three"), pr["intro"])

    hw = p["how"]
    steps = "".join(
        f'<li class="card step"><span class="step-n" aria-hidden="true">{i + 1}</span><h3>{st["title"]}</h3><div class="prose muted">{paras(st["body"])}</div></li>'
        for i, st in enumerate(hw["steps"])
    )
    how = section("how", hw["heading"], f'<ol class="steps">{steps}</ol>', hw["intro"])

    ft = p["features"]
    features = section("features", ft["heading"], cards(ft["items"]), ft["intro"])

    sc = p["screens"]
    figs = "".join(
        f'<figure class="phone-shot">{shot(it["image"], it["alt"], *SHOT_SIZES[it["image"]])}<figcaption><strong>{it["title"]}</strong><span class="muted">{it["caption"]}</span></figcaption></figure>'
        for it in sc["items"]
    )
    screens = section("screens", sc["heading"], f'<div class="shots">{figs}</div><p class="caption-strong center">{sc["note"]}</p>', sc["intro"])

    nw = p["networks"]
    networks = section("networks", nw["heading"], cards(nw["items"], "cards two"), nw["intro"])

    rl = p["relay"]
    can = "".join(f"<li>{x}</li>" for x in rl["can"])
    cannot = "".join(f"<li>{x}</li>" for x in rl["cannot"])
    relay_body = f"""<div class="cards two">
  <article class="card"><h3>{rl["can_heading"]}</h3><ul class="ticks warn">{can}</ul></article>
  <article class="card"><h3>{rl["cannot_heading"]}</h3><ul class="ticks calm">{cannot}</ul></article>
</div>
<p class="center"><a class="btn ghost" href="{page_path(lang, "privacy")}#relay">{rl["more_privacy"]}</a>
<a class="btn ghost" href="{page_path(lang, "self-hosting")}">{rl["more_selfhost"]}</a></p>"""
    relay = section("relay", f'{rl["heading"]} {pill(s["ui"]["in_development"], "warn")}', relay_body, rl["intro"])

    cta = p["cta"]
    final = f"""<section class="band" aria-labelledby="cta-h"><div class="page"><div class="card cta">
  {logo(56)}<div class="stack-s grow"><h2 id="cta-h">{cta["heading"]}</h2><p class="muted">{cta["body"]}</p></div>
  <div class="row wrap"><a class="btn primary big" href="{page_path(lang, "download")}">{cta["download"]}</a>
  <a class="btn ghost big" href="{REPO}">{cta["source"]}</a></div>
</div></div></section>"""
    return hero + promise + how + features + screens + networks + relay + final


def page_header(p):
    lead = f'<p class="lead muted">{p["lead"]}</p>' if p.get("lead") else ""
    return f'<section class="page-head"><div class="page stack-s"><h1>{p["title"]}</h1>{lead}</div></section>'


def asset_links(key, label_fmt, ui):
    out = []
    for kind, arch, name in ASSETS[key]:
        out.append(
            f'<a class="btn dl" href="{DOWNLOAD}/{name}"><span class="dl-kind">{kind}</span>'
            f'<span class="dl-arch">{e(arch)}</span><span class="sr-only"> ({e(label_fmt)})</span></a>'
        )
    return f'<div class="dl-row">{"".join(out)}</div>'


def download(s, lang):
    p = s["pages"]["download"]
    ui = s["ui"]
    blocks = []
    for pl in p["platforms"]:
        key = pl["key"]
        req = "".join(f"<li>{x}</li>" for x in pl["requirements"])
        notes = paras(pl.get("notes", []))
        soon = pl.get("status")
        if key == "ios":
            links = f'<p><span class="pill warn">{ui["coming_soon"]}</span></p>'
        elif key == "browser":
            links = (
                asset_links("web", pl["name"], ui)
                + f'<p class="row wrap"><code>{e(BASE_URL.split("//")[1])}/app/</code>{pill(ui["coming_soon"], "warn")}</p>'
            )
        else:
            links = asset_links(key, pl["name"], ui)
        status = pill(soon, "warn") if soon else ""
        blocks.append(
            f'<article class="card platform" id="{key}"><div class="row-between wrap"><h2>{pl["name"]}</h2>{status}</div>'
            f'<p class="muted">{pl["roles"]}</p>{links}'
            f'<h3>{p["requirements_heading"]}</h3><ul class="plain-list">{req}</ul>'
            f'<div class="prose muted small">{notes}</div></article>'
        )
    v = p["verify"]
    verify = (
        f'<article class="card stack" id="verify"><h2>{v["heading"]}</h2><div class="prose muted">{paras(v["body"])}</div>'
        f'{code(v["code"])}{asset_links("sums", v["heading"], ui)}</article>'
    )
    src = p["source"]
    source = (
        f'<article class="card stack" id="source"><h2>{src["heading"]}</h2><div class="prose muted">{paras(src["body"])}</div>'
        f'{code(src["code"])}</article>'
    )
    all_rel = f'<p class="center"><a class="btn primary big" href="{LATEST}">{p["all_releases"]}</a></p>'
    intro = f'<div class="card note"><div class="prose">{paras(p["notice"])}</div></div>'
    return (
        page_header(p)
        + f'<div class="page stack wide-gap">{intro}<div class="platforms">{"".join(blocks)}</div>{all_rel}{verify}{source}</div>'
    )


def prose_page(s, lang, key):
    p = s["pages"][key]
    parts = []
    toc = []
    for sec in p["sections"]:
        sid = sec["id"]
        toc.append(f'<li><a href="#{sid}">{sec["heading"]}</a></li>')
        body = paras(sec.get("body", []))
        if sec.get("list"):
            body += '<ul class="plain-list">' + "".join(f"<li>{x}</li>" for x in sec["list"]) + "</ul>"
        if sec.get("table"):
            t = sec["table"]
            head = "".join(f'<th scope="col">{c}</th>' for c in t["head"])
            rows = "".join(
                "<tr>" + "".join((f'<th scope="row">{c}</th>' if i == 0 else f"<td>{c}</td>") for i, c in enumerate(r)) + "</tr>"
                for r in t["rows"]
            )
            body += f'<div class="table-wrap"><table><caption class="sr-only">{sec["heading"]}</caption><thead><tr>{head}</tr></thead><tbody>{rows}</tbody></table></div>'
        if sec.get("code"):
            body += code(sec["code"])
        if sec.get("after"):
            body += paras(sec["after"])
        tag = pill(sec["pill"], "warn") if sec.get("pill") else ""
        parts.append(
            f'<section class="card prose-card" id="{sid}" aria-labelledby="{sid}-h"><div class="row-between wrap"><h2 id="{sid}-h">{sec["heading"]}</h2>{tag}</div>'
            f'<div class="prose">{body}</div></section>'
        )
    aside = f'<nav class="toc card" aria-label="{e(s["ui"]["toc"])}"><h2 class="caption-strong">{s["ui"]["toc"]}</h2><ul>{"".join(toc)}</ul></nav>'
    return page_header(p) + f'<div class="page with-toc">{aside}<div class="stack">{"".join(parts)}</div></div>'


def faq(s, lang):
    p = s["pages"]["faq"]
    items = "".join(
        f'<details class="card faq-item" id="q{i + 1}"><summary><h2>{q["q"]}</h2></summary><div class="prose muted">{paras(q["a"])}</div></details>'
        for i, q in enumerate(p["items"])
    )
    return page_header(p) + f'<div class="page stack faq">{items}</div>'


def render(s, lang, page):
    if page == "home":
        return home(s, lang)
    if page == "download":
        return download(s, lang)
    if page == "faq":
        return faq(s, lang)
    return prose_page(s, lang, page)


def substitute(text: str, lang: str) -> str:
    """Expands {link:page}, {repo}, {latest} and {base} in strings."""
    for k in PAGES:
        text = text.replace("{link:" + k + "}", page_path(lang, k))
    return text.replace("{repo}", REPO).replace("{latest}", LATEST).replace("{base}", BASE_URL)


def load(lang):
    raw = (HERE / "i18n" / f"{lang}.json").read_text(encoding="utf-8")
    return json.loads(substitute(raw, lang))


# ---------------------------------------------------------------- link check


class _Links(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links: list[str] = []
        self.ids: set[str] = set()

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if a.get("id"):
            self.ids.add(a["id"])
        for k in ("href", "src"):
            if a.get(k):
                self.links.append(a[k])


def check_links(out: Path) -> list[str]:
    """Every site-internal href/src must name an existing file and anchor."""
    parsed = {}
    for f in out.rglob("*.html"):
        p = _Links()
        p.feed(f.read_text(encoding="utf-8"))
        parsed[f] = p
    bad = []
    for f, p in parsed.items():
        for link in p.links:
            if link.startswith(("http://", "https://", "data:", "mailto:")):
                continue
            path, _, frag = link.partition("#")
            target = f if not path else out / path.lstrip("/")
            if path.endswith("/"):
                target = target / "index.html"
            if not target.exists():
                bad.append(f"{f.relative_to(out)}: {link}")
            elif frag and target in parsed and frag not in parsed[target].ids:
                bad.append(f"{f.relative_to(out)}: {link} (no such anchor)")
    return bad


STRINGS: dict = {}


def main(out: Path) -> None:
    for lang in LANGS:
        STRINGS[lang] = load(lang)
    ref = shape(STRINGS[LANGS[0]])
    for lang in LANGS[1:]:
        d = diff_shape(ref, shape(STRINGS[lang]))
        if d:
            sys.exit(f"i18n/{lang}.json is out of sync with i18n/{LANGS[0]}.json:\n  " + "\n  ".join(d))

    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    for name in ("nestwatch.css", "logo.svg"):
        shutil.copy(ROOT / "design" / name, out / name)
    shutil.copy(ROOT / "design" / "logo.svg", out / "favicon.svg")
    for name in ("site.css", "site.js"):
        shutil.copy(HERE / name, out / name)
    shutil.copytree(HERE / "img", out / "img")

    urls = []
    for lang in LANGS:
        for page in PAGES:
            path = page_path(lang, page)
            target = out / path.strip("/") / "index.html"
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(layout(STRINGS[lang], lang, page, render(STRINGS[lang], lang, page)), encoding="utf-8")
            urls.append((lang, page))

    not_found = STRINGS[LANGS[0]]["ui"]["not_found"]
    (out / "404.html").write_text(
        layout(STRINGS[LANGS[0]], LANGS[0], "home", f'<section class="page-head"><div class="page stack-s"><h1>{not_found["title"]}</h1>'
               f'<p class="lead muted">{not_found["body"]}</p><p><a class="btn primary" href="/">{not_found["home"]}</a></p></div></section>')
        .replace('<link rel="canonical"', '<meta name="robots" content="noindex"><link rel="canonical"', 1),
        encoding="utf-8",
    )

    today = date.today().isoformat()
    entries = []
    for lang, page in urls:
        alts = "".join(
            f'<xhtml:link rel="alternate" hreflang="{x}" href="{BASE_URL}{page_path(x, page)}"/>' for x in LANGS
        )
        entries.append(f"<url><loc>{BASE_URL}{page_path(lang, page)}</loc><lastmod>{today}</lastmod>{alts}</url>")
    (out / "sitemap.xml").write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">\n'
        + "\n".join(entries)
        + "\n</urlset>\n",
        encoding="utf-8",
    )
    (out / "robots.txt").write_text(f"User-agent: *\nAllow: /\n\nSitemap: {BASE_URL}/sitemap.xml\n", encoding="utf-8")
    bad = check_links(out)
    if bad:
        sys.exit("broken internal links:\n  " + "\n  ".join(bad))
    print(f"nestwatch site in {out} ({len(urls)} pages, internal links checked)")


if __name__ == "__main__":
    main(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "nestwatch-site")
