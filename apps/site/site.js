// SPDX-License-Identifier: AGPL-3.0-or-later
// Nestwatch website: theme toggle and the phone menu. Nothing else, and no
// network requests. The only stored value is the chosen theme
// (localStorage "nestwatch-site-theme"); no cookies.
(function () {
  var KEY = "nestwatch-site-theme";
  var root = document.documentElement;

  function stored() {
    try { return localStorage.getItem(KEY); } catch (e) { return null; }
  }
  function store(v) {
    try { localStorage.setItem(KEY, v); } catch (e) { /* private mode */ }
  }
  function systemTheme() {
    return window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches ? "day" : "night";
  }
  function current() {
    return root.getAttribute("data-theme") || systemTheme();
  }

  // Before first paint, so a chosen theme never flashes the other one.
  root.classList.add("js");
  var saved = stored();
  if (saved === "day" || saved === "night") root.setAttribute("data-theme", saved);

  function sync(btn) {
    var theme = current();
    var label = btn.getAttribute(theme === "day" ? "data-label-day" : "data-label-night");
    btn.setAttribute("aria-label", label);
    btn.setAttribute("title", label);
  }

  document.addEventListener("DOMContentLoaded", function () {
    var btn = document.querySelector(".theme-toggle");
    if (btn) {
      sync(btn);
      btn.addEventListener("click", function () {
        var next = current() === "day" ? "night" : "day";
        root.setAttribute("data-theme", next);
        store(next);
        sync(btn);
      });
      if (window.matchMedia) {
        var mq = window.matchMedia("(prefers-color-scheme: light)");
        var onChange = function () { sync(btn); };
        if (mq.addEventListener) mq.addEventListener("change", onChange);
      }
    }

    var menu = document.querySelector(".menu-toggle");
    var nav = document.getElementById("site-nav");
    if (menu && nav) {
      menu.addEventListener("click", function () {
        var open = menu.getAttribute("aria-expanded") !== "true";
        menu.setAttribute("aria-expanded", open ? "true" : "false");
        nav.classList.toggle("open", open);
      });
      document.addEventListener("keydown", function (ev) {
        if (ev.key === "Escape" && nav.classList.contains("open")) {
          menu.setAttribute("aria-expanded", "false");
          nav.classList.remove("open");
          menu.focus();
        }
      });
    }
  });
})();
