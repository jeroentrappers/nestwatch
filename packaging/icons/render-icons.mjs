#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Renders the desktop app icons from design/logo.svg: the logo on a rounded
// night-ink square. Writes nestwatch-<size>.png here. Needs Node 22+ and
// Chrome. The committed set keeps 16, 32, 128, 256, 512 and 1024; rebuild the
// Windows icon from the rendered sizes with ImageMagick:
//   magick nestwatch-{16,32,48,64,128,256}.png nestwatch.ico
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { launch } from "../../apps/site/tools/cdp.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const logo = readFileSync(join(here, "..", "..", "design", "logo.svg"), "utf8");
const b = await launch();
await b.page("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } });
for (const size of [16, 32, 48, 64, 128, 256, 512, 1024]) {
  await b.page("Emulation.setDeviceMetricsOverride", { width: size, height: size, deviceScaleFactor: 1, mobile: false });
  // macOS-style proportions: 80 % square with 22 % corner radius.
  const sq = Math.round(size * (size <= 32 ? 1 : 0.8));
  const pad = Math.round((size - sq) / 2);
  const mark = Math.round(sq * 0.78);
  await b.goto("data:text/html," + encodeURIComponent(
    `<html style="background:transparent"><body style="margin:0;width:${size}px;height:${size}px;background:transparent">` +
    `<div style="position:absolute;left:${pad}px;top:${pad}px;width:${sq}px;height:${sq}px;border-radius:${Math.round(sq * 0.22)}px;` +
    `background:linear-gradient(160deg,#212433,#0f1017);display:grid;place-items:center">` +
    logo.replace("<svg ", `<svg width="${mark}" height="${mark}" `) + "</div></body></html>"));
  const { data } = await b.page("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(here, `nestwatch-${size}.png`), Buffer.from(data, "base64"));
}
await b.close();
console.log("icons rendered");
