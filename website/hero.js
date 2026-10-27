// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// The home page's background: one GL ES 2.0 fragment shader on the visitor's
// own GPU, the kind of program YakoGL compiles for an embedded one. Without
// WebGL, or if anything fails, the canvas stays empty and the band's colour
// behind it shows. With prefers-reduced-motion it draws one still frame.
// data-palette="night" keeps the dark palette whatever the colour scheme.
(function () {
  "use strict";
  var canvas = document.getElementById("hero-canvas");
  if (!canvas) return;
  var gl = null;
  try {
    gl = canvas.getContext("webgl", { alpha: true, antialias: false, depth: false, premultipliedAlpha: false });
  } catch (e) { gl = null; }
  if (!gl) return;

  var VS = "attribute vec2 p;\nvoid main() { gl_Position = vec4(p, 0.0, 1.0); }\n";
  var FS = [
    "precision mediump float;",
    "uniform float t;",
    "uniform vec2 res;",
    "uniform vec3 c0;",
    "uniform vec3 c1;",
    "uniform vec3 c2;",
    "void main() {",
    "  vec2 q = (gl_FragCoord.xy - 0.5 * res) / res.y;",
    "  float v = sin(3.1 * q.x + t * 0.50) + sin(2.3 * q.y - t * 0.35)",
    "          + sin(2.0 * (q.x + q.y) + t * 0.70) + sin(7.0 * length(q) - t * 0.60);",
    "  v *= 0.25;",
    "  vec3 nacre = mix(c1, c2, 0.5 + 0.5 * sin(v * 3.14159 + t * 0.25));",
    "  float a = 0.35 + 0.35 * v;",
    "  gl_FragColor = vec4(mix(c0, nacre, a), 1.0);",
    "}"
  ].join("\n");

  function compile(type, src) {
    var s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
  }
  var vs = compile(gl.VERTEX_SHADER, VS), fs = compile(gl.FRAGMENT_SHADER, FS);
  if (!vs || !fs) return;
  var prog = gl.createProgram();
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return;
  gl.useProgram(prog);

  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  var loc = gl.getAttribLocation(prog, "p");
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

  var u = {};
  ["t", "res", "c0", "c1", "c2"].forEach(function (name) {
    u[name] = gl.getUniformLocation(prog, name);
  });

  var dark = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  var still = window.matchMedia ? window.matchMedia("(prefers-reduced-motion: reduce)") : null;
  var night = canvas.getAttribute("data-palette") === "night";
  function palette() {
    if (night || (dark && dark.matches)) {
      gl.uniform3f(u.c0, 0.07, 0.08, 0.10);
      gl.uniform3f(u.c1, 0.16, 0.42, 0.40);
      gl.uniform3f(u.c2, 0.36, 0.30, 0.62);
    } else {
      gl.uniform3f(u.c0, 0.96, 0.96, 0.95);
      gl.uniform3f(u.c1, 0.62, 0.85, 0.80);
      gl.uniform3f(u.c2, 0.80, 0.76, 0.95);
    }
  }
  palette();

  function resize() {
    var r = Math.min(window.devicePixelRatio || 1, 1.5);
    var w = Math.max(1, Math.round(canvas.clientWidth * r));
    var h = Math.max(1, Math.round(canvas.clientHeight * r));
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    gl.viewport(0, 0, w, h);
    gl.uniform2f(u.res, w, h);
  }

  var start = Date.now(), raf = 0, lost = false;
  function frame() {
    raf = 0;
    if (lost) return;
    resize();
    gl.uniform1f(u.t, (Date.now() - start) / 1000);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    if (!(still && still.matches) && !document.hidden) raf = window.requestAnimationFrame(frame);
  }
  function kick() {
    if (!raf && !lost) raf = window.requestAnimationFrame(frame);
  }

  canvas.addEventListener("webglcontextlost", function (e) { e.preventDefault(); lost = true; });
  document.addEventListener("visibilitychange", kick);
  window.addEventListener("resize", kick);
  [dark, still].forEach(function (mq) {
    if (!mq) return;
    var cb = function () { palette(); kick(); };
    if (mq.addEventListener) mq.addEventListener("change", cb);
    else if (mq.addListener) mq.addListener(cb);
  });
  kick();
})();
