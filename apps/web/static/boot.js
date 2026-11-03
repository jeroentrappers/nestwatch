// SPDX-License-Identifier: AGPL-3.0-or-later
// Loads the Lull browser listener (wasm). An external file because the
// station's CSP allows scripts from 'self' only (NET-5).
import init from "./lull-web.js";

init({ module_or_path: new URL("lull-web_bg.wasm", import.meta.url) }).catch((e) => {
  console.error("Lull failed to start", e);
  const p = document.createElement("p");
  p.textContent = "Lull could not start in this browser.";
  document.body.append(p);
});
