// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// The home page's motion: the scroll progress bar under the header, and a
// fade-in for blocks that start below the fold. Without this script, or with
// prefers-reduced-motion, every block is simply shown.
(function () {
  "use strict";
  var fill = document.getElementById("progress-fill");
  var raf = 0;
  function progress() {
    raf = 0;
    var max = document.documentElement.scrollHeight - window.innerHeight;
    if (fill) fill.style.width = (max > 0 ? Math.min(100, (window.scrollY / max) * 100) : 0) + "%";
  }
  function onScroll() {
    if (!raf) raf = window.requestAnimationFrame(progress);
  }
  window.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", onScroll);
  progress();

  var still = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (still || !("IntersectionObserver" in window)) return;
  var els = document.querySelectorAll(
    ".home .section-head, .reasons li, .cols3 > div, .stages li, .doclist li, .band .tiles, .band-title, .h-term.on-night");
  var io = new IntersectionObserver(function (entries) {
    entries.forEach(function (e) {
      if (!e.isIntersecting) return;
      e.target.classList.remove("pre");
      io.unobserve(e.target);
    });
  }, { rootMargin: "0px 0px -8% 0px" });
  for (var i = 0; i < els.length; i++) {
    var el = els[i];
    el.classList.add("reveal");
    // Only blocks below the fold are hidden first, so nothing in view blinks.
    if (el.getBoundingClientRect().top > window.innerHeight) {
      el.classList.add("pre");
      io.observe(el);
    }
  }
})();
