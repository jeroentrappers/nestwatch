// SPDX-License-Identifier: AGPL-3.0-or-later
// Loads the Nestwatch browser listener (wasm). An external file because the
// station's CSP allows scripts from 'self' only (NET-5).
import init from "./nestwatch-web.js";

init({ module_or_path: new URL("nestwatch-web_bg.wasm", import.meta.url) }).catch((e) => {
  console.error("Nestwatch failed to start", e);
  const p = document.createElement("p");
  p.textContent = "Nestwatch could not start in this browser.";
  document.body.append(p);
});
