#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Regenerates the committed images of the website from the design system:
// img/og-<lang>.png (1200x630 Open Graph cards) and img/icon-180.png
// (apple-touch-icon). Needs Node 22+ and google-chrome (or $CHROME).
//
//   node apps/site/tools/render-images.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { launch } from "./cdp.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const site = join(here, "..");
const b = await launch();
await b.page("Emulation.setDeviceMetricsOverride", { width: 1200, height: 630, deviceScaleFactor: 1, mobile: false });
for (const lang of ["en", "nl"]) {
  const s = JSON.parse(readFileSync(join(site, "i18n", `${lang}.json`), "utf8"));
  const hero = s.pages.home.hero;
  await b.goto(pathToFileURL(join(here, "og.html")).href);
  await b.eval(`(() => {
    document.documentElement.lang = ${JSON.stringify(lang)};
    document.getElementById("title").textContent = ${JSON.stringify(s.pages.home.title)};
    const kinds = ["calm", "voice", "cam"];
    document.getElementById("pills").innerHTML = ${JSON.stringify(hero.pills)}
      .map((t, i) => '<span class="pill ' + kinds[i] + '">' + t + '</span>').join("");
    return document.fonts.ready.then(() => Promise.all([...document.images].map(i => i.decode())));
  })()`);
  const { data } = await b.page("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(site, "img", `og-${lang}.png`), Buffer.from(data, "base64"));
}
// Touch icon: the logo on the night page colour with some breathing room.
await b.page("Emulation.setDeviceMetricsOverride", { width: 180, height: 180, deviceScaleFactor: 1, mobile: false });
const logo = readFileSync(join(site, "..", "..", "design", "logo.svg"), "utf8");
await b.goto("data:text/html," + encodeURIComponent(
  `<body style="margin:0;background:#0f1017;display:grid;place-items:center;width:180px;height:180px">${logo.replace("<svg ", '<svg width="132" height="132" ')}</body>`));
const { data } = await b.page("Page.captureScreenshot", { format: "png" });
writeFileSync(join(site, "img", "icon-180.png"), Buffer.from(data, "base64"));
await b.close();
console.log("rendered og-en.png, og-nl.png, icon-180.png");
