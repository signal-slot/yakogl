// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Renders the figures of data.js into the pages. Markup hooks:
//   <span data-fig="id">        the value
//   <span data-prov="id">       "measured on <commit>, <date>" with a link to the record
//   <tbody data-status="board"> the per-board rows of status.html
//   <div data-tiles="kind">     the highlight tiles of index.html
//   <span data-meta="checked">  the main commit and date data.js was checked against
(function () {
  "use strict";
  var D = window.YAKOGL_DATA;
  if (!D) return;
  document.documentElement.classList.add("has-data");

  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }
  function num(v) {
    return typeof v === "number" ? v.toLocaleString("en-US") : esc(v);
  }
  function blob(f) {
    return D.repo + "/blob/" + f.provenance.recordCommit + "/" + f.source;
  }

  function value(f) {
    var v = f.value;
    if (v && typeof v === "object") {
      var parts = [num(v.pass) + " pass", num(v.fail) + " fail"];
      if (v.notSupported !== undefined) parts.push(num(v.notSupported) + " not supported");
      if (v.warnings !== undefined) parts.push(num(v.warnings) + " warnings");
      if (v.noResult !== undefined) parts.push(num(v.noResult) + " without a result");
      return parts.join(" · ");
    }
    return num(v);
  }

  function prov(f) {
    var bits = [];
    if (f.commit) {
      bits.push("build <code>" + esc(f.commit) + "</code>");
      if (f.provenance.kind === "pre-publication") bits.push("pre-publication build");
      else if (!f.onMain) bits.push("branch build");
    } else {
      bits.push(f.provenance.kind === "external" ? "external measurement" : "build commit not recorded");
    }
    bits.push(f.date ? esc(f.date) : "date not stated");
    var src = '<a href="' + esc(blob(f)) + '" title="' + esc(f.section || "") + '">Public record: ' + esc(f.source) + "</a>";
    var out = bits.join(", ") + " · " + src;
    if (f.unverified) out += ' · <span class="unv" title="part of the provenance could not be checked">unverified</span>';
    return out;
  }

  function fill(attr, fn) {
    var els = document.querySelectorAll("[" + attr + "]");
    for (var i = 0; i < els.length; i++) {
      var f = D.figures[els[i].getAttribute(attr)];
      els[i].innerHTML = f ? fn(f) : '<span class="unv">missing figure</span>';
    }
  }
  fill("data-fig", value);
  fill("data-prov", prov);
  fill("data-note", function (f) { return esc(f.note || ""); });

  var meta = document.querySelectorAll('[data-meta="checked"]');
  for (var m = 0; m < meta.length; m++) {
    meta[m].innerHTML = '<a href="' + esc(D.repo + "/commit/" + D.checkedAgainst.commit) + '"><code>' +
      esc(D.checkedAgainst.commit.slice(0, 9)) + "</code></a>, " + esc(D.checkedAgainst.date);
  }

  // status.html: one table per board.
  var ROWS = [
    ["dEQP-GLES2", "deqp2"],
    ["dEQP-GLES2, multisample (4x image)", "deqp2", ".ms4"],
    ["gl_test", "gltest"],
    ["glmark2-es2", "glmark2"],
    ["glmark2 public reference", "glmark2ref"],
    ["Boot from own storage", "boot"],
    ["dEQP-GLES3", "deqp3"]
  ];
  var tbodies = document.querySelectorAll("tbody[data-status]");
  for (var t = 0; t < tbodies.length; t++) {
    var b = tbodies[t].getAttribute("data-status"), html = "";
    for (var r = 0; r < ROWS.length; r++) {
      var f = D.figures[ROWS[r][1] + "." + b + (ROWS[r][2] || "")];
      if (!f) continue;
      html += '<tr class="st-' + esc(f.status) + '"><th scope="row">' + esc(ROWS[r][0]) +
        '</th><td><span class="badge">' + esc(f.status) + '</span> <strong>' + value(f) +
        "</strong> <span class=\"unit\">" + esc(f.unit) + "</span>" +
        (f.note ? '<div class="fnote">' + esc(f.note) + "</div>" : "") +
        '</td><td class="prov">' + prov(f) + "</td></tr>";
    }
    tbodies[t].innerHTML = html;
  }

  // index.html: highlight tiles.
  var tiles = document.querySelectorAll("[data-tiles]");
  for (var k = 0; k < tiles.length; k++) {
    var kind = tiles[k].getAttribute("data-tiles"), out = "";
    for (var j = 0; j < D.boards.length; j++) {
      var bd = D.boards[j], fg = D.figures[kind + "." + bd.id];
      if (!fg) continue;
      var big = fg.value && typeof fg.value === "object" ? num(fg.value.fail) + " fail" : num(fg.value);
      var sub = fg.value && typeof fg.value === "object" ? num(fg.value.pass) + " of " + num(fg.value.total) + " pass" : esc(fg.unit);
      out += '<div class="tile st-' + esc(fg.status) + '"><div class="tile-board">' + esc(bd.short) +
        ' <span class="tile-gpu">' + esc(bd.gpu) + '</span></div><div class="tile-big">' + big +
        '</div><div class="tile-sub">' + sub + '</div><div class="prov">' + prov(fg) + "</div></div>";
    }
    tiles[k].innerHTML = out;
  }
})();
