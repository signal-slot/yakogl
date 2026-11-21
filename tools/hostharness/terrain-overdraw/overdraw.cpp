// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Overdraw of glmark2 terrain's terrain draw (V3D job 3), counted exactly on
// the host by a software rasteriser. See README.md.
//
// Reproduces, from glmark2's sources (scene-terrain.cpp, scene-terrain/*.cpp,
// data/shaders/terrain-noise.frag, terrain.vert): the 256x256 height map (the
// simplex noise shader, quantised to RGBA8), the 6000x6000 grid of 256x256
// cells (two triangles each, glmark2's vertex order), terrain.vert's
// displacement and the camera, then rasterises the 131,072 triangles in draw
// order at 800x600 with back-face culling and a LEQUAL depth test, counting
//   rasterized: fragments the rasteriser makes (what the shader would run on
//               with early Z off)
//   covered:    pixels with at least one fragment
//   passed:     fragments that pass LEQUAL when they arrive (what an exact
//               early Z would shade)
// Usage: overdraw <seconds> [<seconds> ...]   (glmark2's realTime_.elapsed()
// at the frame; uvOffset of the height map is (0.05 * seconds, 0))
// OVERDRAW_FLOAT_DEPTH=1: compare depth as float instead of 24 bits.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <vector>

typedef float f32;
struct V3 { f32 x, y, z; };

static f32 fl(f32 x) { return floorf(x); }
static f32 glmod(f32 x, f32 y) { return x - y * fl(x / y); }
static f32 stp(f32 e, f32 x) { return x < e ? 0.f : 1.f; }

// ---- terrain-noise.frag (snoise, abs sum over four octaves), f32 ----
static void permute(const f32 x[4], f32 o[4]) {
  for (int k = 0; k < 4; k++) o[k] = glmod(((x[k] * 34.f) + 1.f) * x[k], 289.f);
}
static f32 dot3(V3 a, V3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
static f32 snoise(V3 v) {
  const f32 Cx = 1.f / 6.f, Cy = 1.f / 3.f;
  f32 d = (v.x + v.y + v.z) * Cy;
  V3 i = {fl(v.x + d), fl(v.y + d), fl(v.z + d)};
  f32 e = (i.x + i.y + i.z) * Cx;
  V3 x0 = {v.x - i.x + e, v.y - i.y + e, v.z - i.z + e};
  // g = step(x0.yzx, x0.xyz)
  V3 g = {stp(x0.y, x0.x), stp(x0.z, x0.y), stp(x0.x, x0.z)};
  V3 l = {1.f - g.x, 1.f - g.y, 1.f - g.z};
  V3 lz = {l.z, l.x, l.y};  // l.zxy
  V3 i1 = {fminf(g.x, lz.x), fminf(g.y, lz.y), fminf(g.z, lz.z)};
  V3 i2 = {fmaxf(g.x, lz.x), fmaxf(g.y, lz.y), fmaxf(g.z, lz.z)};
  V3 x1 = {x0.x - i1.x + Cx, x0.y - i1.y + Cx, x0.z - i1.z + Cx};
  V3 x2 = {x0.x - i2.x + 2.f * Cx, x0.y - i2.y + 2.f * Cx, x0.z - i2.z + 2.f * Cx};
  V3 x3 = {x0.x - 1.f + 3.f * Cx, x0.y - 1.f + 3.f * Cx, x0.z - 1.f + 3.f * Cx};
  i = {glmod(i.x, 289.f), glmod(i.y, 289.f), glmod(i.z, 289.f)};
  f32 a[4] = {i.z + 0.f, i.z + i1.z, i.z + i2.z, i.z + 1.f}, b[4];
  permute(a, b);
  f32 c[4], dd[4];
  f32 yy[4] = {0.f, i1.y, i2.y, 1.f}, xx[4] = {0.f, i1.x, i2.x, 1.f};
  for (int k = 0; k < 4; k++) c[k] = b[k] + i.y + yy[k];
  permute(c, dd);
  f32 e2[4], p[4];
  for (int k = 0; k < 4; k++) e2[k] = dd[k] + i.x + xx[k];
  permute(e2, p);
  // ns = n_ * D.wyz - D.xzx, D = (0, .5, 1, 2), n_ = 1/7
  const f32 n_ = 1.f / 7.f;
  f32 ns_x = n_ * 2.f - 0.f, ns_y = n_ * 0.5f - 1.f, ns_z = n_ * 1.f - 0.f;
  f32 j[4], xo[4], yo[4], X[4], Y[4], h[4];
  for (int k = 0; k < 4; k++) {
    j[k] = p[k] - 49.f * fl(p[k] * ns_z * ns_z);
    xo[k] = fl(j[k] * ns_z);
    yo[k] = fl(j[k] - 7.f * xo[k]);
    X[k] = xo[k] * ns_x + ns_y;
    Y[k] = yo[k] * ns_x + ns_y;
    h[k] = 1.f - fabsf(X[k]) - fabsf(Y[k]);
  }
  f32 b0[4] = {X[0], X[1], Y[0], Y[1]}, b1[4] = {X[2], X[3], Y[2], Y[3]};
  f32 s0[4], s1[4], sh[4];
  for (int k = 0; k < 4; k++) {
    s0[k] = fl(b0[k]) * 2.f + 1.f;
    s1[k] = fl(b1[k]) * 2.f + 1.f;
    sh[k] = -stp(h[k], 0.f);
  }
  // a0 = b0.xzyw + s0.xzyw * sh.xxyy ; a1 = b1.xzyw + s1.xzyw * sh.zzww
  f32 a0[4] = {b0[0] + s0[0] * sh[0], b0[2] + s0[2] * sh[0],
               b0[1] + s0[1] * sh[1], b0[3] + s0[3] * sh[1]};
  f32 a1[4] = {b1[0] + s1[0] * sh[2], b1[2] + s1[2] * sh[2],
               b1[1] + s1[1] * sh[3], b1[3] + s1[3] * sh[3]};
  V3 p0 = {a0[0], a0[1], h[0]}, p1 = {a0[2], a0[3], h[1]};
  V3 p2 = {a1[0], a1[1], h[2]}, p3 = {a1[2], a1[3], h[3]};
  f32 nr[4] = {dot3(p0, p0), dot3(p1, p1), dot3(p2, p2), dot3(p3, p3)};
  for (int k = 0; k < 4; k++) nr[k] = 1.79284291400159f - 0.85373472095314f * nr[k];
  auto sc = [](V3 q, f32 s) { return V3{q.x * s, q.y * s, q.z * s}; };
  p0 = sc(p0, nr[0]); p1 = sc(p1, nr[1]); p2 = sc(p2, nr[2]); p3 = sc(p3, nr[3]);
  f32 m[4] = {fmaxf(0.6f - dot3(x0, x0), 0.f), fmaxf(0.6f - dot3(x1, x1), 0.f),
              fmaxf(0.6f - dot3(x2, x2), 0.f), fmaxf(0.6f - dot3(x3, x3), 0.f)};
  for (int k = 0; k < 4; k++) m[k] *= m[k];
  f32 q[4] = {dot3(p0, x0), dot3(p1, x1), dot3(p2, x2), dot3(p3, x3)};
  f32 s = 0;
  for (int k = 0; k < 4; k++) s += m[k] * m[k] * q[k];
  return 42.f * s;
}
static f32 surface3(V3 c) {
  f32 n = 0;
  n += 1.0f * fabsf(snoise(c));
  n += 0.5f * fabsf(snoise({c.x * 2, c.y * 2, c.z * 2}));
  n += 0.25f * fabsf(snoise({c.x * 4, c.y * 4, c.z * 4}));
  n += 0.125f * fabsf(snoise({c.x * 8, c.y * 8, c.z * 8}));
  return n;
}

// 256x256 RGBA8 red channel the noise pass leaves (texture-renderer's quad:
// vUv = uvScale * (pos.xy * .5 + .5) + uvOffset at each pixel centre).
static const int HM = 256;
static std::vector<uint8_t> height_map(f32 off_u) {
  std::vector<uint8_t> t(HM * HM);
  const f32 uvs = 1.5f;
  for (int y = 0; y < HM; y++)
    for (int x = 0; x < HM; x++) {
      f32 px = ((x + 0.5f) / HM) * 2.f - 1.f, py = ((y + 0.5f) / HM) * 2.f - 1.f;
      f32 u = uvs * (px * 0.5f + 0.5f) + off_u, v = uvs * (py * 0.5f + 0.5f);
      f32 n = surface3({u, uvs - v, -1.0f});
      n = fminf(fmaxf(n, 0.f), 1.f);
      t[y * HM + x] = (uint8_t)lrintf(n * 255.f);
    }
  return t;
}
// texture2D(tDisplacement, uv).x: LINEAR, CLAMP_TO_EDGE, level 0.
static f32 sample_h(const std::vector<uint8_t> &t, f32 u, f32 v) {
  f32 fx = u * HM - 0.5f, fy = v * HM - 0.5f;
  f32 x0 = fl(fx), y0 = fl(fy), ax = fx - x0, ay = fy - y0;
  auto at = [&](int x, int y) {
    x = std::min(std::max(x, 0), HM - 1);
    y = std::min(std::max(y, 0), HM - 1);
    return t[y * HM + x] / 255.f;
  };
  int xi = (int)x0, yi = (int)y0;
  return (at(xi, yi) * (1 - ax) + at(xi + 1, yi) * ax) * (1 - ay) +
         (at(xi, yi + 1) * (1 - ax) + at(xi + 1, yi + 1) * ax) * ay;
}

// ---- matrices (libmatrix: gluLookAt, gluPerspective, rotate, translate) ----
struct M4 { double m[4][4]; };
static M4 ident() { M4 r{}; for (int i = 0; i < 4; i++) r.m[i][i] = 1; return r; }
static M4 mul(const M4 &a, const M4 &b) {
  M4 r{};
  for (int i = 0; i < 4; i++)
    for (int j = 0; j < 4; j++)
      for (int k = 0; k < 4; k++) r.m[i][j] += a.m[i][k] * b.m[k][j];
  return r;
}
static M4 translate(double x, double y, double z) {
  M4 r = ident(); r.m[0][3] = x; r.m[1][3] = y; r.m[2][3] = z; return r;
}
static M4 rotx(double deg) {
  double a = deg * M_PI / 180, c = cos(a), s = sin(a);
  M4 r = ident(); r.m[1][1] = c; r.m[1][2] = -s; r.m[2][1] = s; r.m[2][2] = c; return r;
}
static M4 lookat(double ex, double ey, double ez, double cx, double cy, double cz,
                 double ux, double uy, double uz) {
  double f[3] = {cx - ex, cy - ey, cz - ez};
  double fn = sqrt(f[0] * f[0] + f[1] * f[1] + f[2] * f[2]);
  for (double &q : f) q /= fn;
  double s[3] = {f[1] * uz - f[2] * uy, f[2] * ux - f[0] * uz, f[0] * uy - f[1] * ux};
  double sl = sqrt(s[0] * s[0] + s[1] * s[1] + s[2] * s[2]);
  for (double &q : s) q /= sl;
  double u[3] = {s[1] * f[2] - s[2] * f[1], s[2] * f[0] - s[0] * f[2], s[0] * f[1] - s[1] * f[0]};
  M4 r = ident();
  for (int k = 0; k < 3; k++) { r.m[0][k] = s[k]; r.m[1][k] = u[k]; r.m[2][k] = -f[k]; }
  return mul(r, translate(-ex, -ey, -ez));
}
static M4 perspective(double fovy, double aspect, double zn, double zf) {
  double f = 1 / tan(fovy * M_PI / 180 / 2);
  M4 p{};
  p.m[0][0] = f / aspect; p.m[1][1] = f;
  p.m[2][2] = (zf + zn) / (zn - zf); p.m[2][3] = 2 * zf * zn / (zn - zf);
  p.m[3][2] = -1;
  return p;
}

static const int W = 800, H = 600;
struct SV { double x, y, z, w; };  // clip space

struct Counts {
  uint64_t tris = 0, culled = 0, drawn = 0, rast = 0, passed = 0, covered = 0, clipped_z = 0;
};

static void raster(const SV v[3], std::vector<float> &depth, std::vector<uint16_t> &hits,
                   Counts &c, bool q24) {
  double X[3], Y[3], Z[3];
  for (int i = 0; i < 3; i++) {
    double iw = 1.0 / v[i].w;
    X[i] = (v[i].x * iw * 0.5 + 0.5) * W;
    Y[i] = (v[i].y * iw * 0.5 + 0.5) * H;
    Z[i] = v[i].z * iw;
  }
  double area = (X[1] - X[0]) * (Y[2] - Y[0]) - (X[2] - X[0]) * (Y[1] - Y[0]);
  if (area <= 0) { c.culled++; return; }  // CCW is front (y up), back culled
  c.drawn++;
  int minx = std::max(0, (int)floor(std::min({X[0], X[1], X[2]})));
  int maxx = std::min(W - 1, (int)ceil(std::max({X[0], X[1], X[2]})));
  int miny = std::max(0, (int)floor(std::min({Y[0], Y[1], Y[2]})));
  int maxy = std::min(H - 1, (int)ceil(std::max({Y[0], Y[1], Y[2]})));
  // top-left fill rule for a pixel centre exactly on an edge
  auto edge_ok = [&](double e, int a, int b) {
    if (e > 0) return true;
    if (e < 0) return false;
    double dx = X[b] - X[a], dy = Y[b] - Y[a];
    return dy > 0 || (dy == 0 && dx < 0);
  };
  for (int py = miny; py <= maxy; py++)
    for (int px = minx; px <= maxx; px++) {
      double cx = px + 0.5, cy = py + 0.5;
      double e0 = (X[2] - X[1]) * (cy - Y[1]) - (Y[2] - Y[1]) * (cx - X[1]);
      double e1 = (X[0] - X[2]) * (cy - Y[2]) - (Y[0] - Y[2]) * (cx - X[2]);
      double e2 = (X[1] - X[0]) * (cy - Y[0]) - (Y[1] - Y[0]) * (cx - X[0]);
      if (!(edge_ok(e0, 1, 2) && edge_ok(e1, 2, 0) && edge_ok(e2, 0, 1))) continue;
      double z = (e0 * Z[0] + e1 * Z[1] + e2 * Z[2]) / area;
      if (z < -1.0 || z > 1.0) { c.clipped_z++; continue; }  // near/far plane clip
      float zw = (float)(z * 0.5 + 0.5);
      if (q24) zw = (float)(floor(zw * 16777215.0 + 0.5) / 16777215.0);
      size_t i = (size_t)py * W + px;
      c.rast++;
      if (hits[i]++ == 0) c.covered++;
      if (zw <= depth[i]) { depth[i] = zw; c.passed++; }
    }
}

int main(int argc, char **argv) {
  if (argc < 2) { fprintf(stderr, "usage: %s <seconds>...\n", argv[0]); return 2; }
  const bool q24 = !getenv("OVERDRAW_FLOAT_DEPTH");
  // scene setup (scene-terrain.cpp)
  M4 view = lookat(-1200, 800, 1200, 0, 0, 0, 0, 1, 0);
  M4 model = mul(translate(0, -125, 0), rotx(-90));
  M4 mv = mul(view, model);
  M4 proj = perspective(40.0, W / (double)H, 2.0, 4000.0);
  // normalMatrix is the inverse transpose of a rigid mv: its upper 3x3 is
  // mv's rotation and its fourth column is zero, so vec4(normal, 1) maps to
  // R * normal; the grid's normal is (0, 0, 1).
  double nv[3] = {mv.m[0][2], mv.m[1][2], mv.m[2][2]};
  const int N = 256;
  const double size = 6000.0, cell = size / N;
  for (int ai = 1; ai < argc; ai++) {
    double secs = atof(argv[ai]);
    auto hm = height_map((f32)(secs * 0.05f));
    std::vector<float> depth((size_t)W * H, 1.0f);
    std::vector<uint16_t> hits((size_t)W * H, 0);
    Counts c;
    for (int i = 0; i < N; i++)
      for (int j = 0; j < N; j++) {
        // make_grid: a = upper left, b = lower left, c = upper right, d = lower right
        double ax = -size / 2 + i * cell, ay = size / 2 - j * cell;
        struct P { double x, y; f32 u, v; };
        f32 u0 = (f32)i / N, u1 = (f32)(i + 1) / N;
        f32 v0 = 1.f - (f32)j / N, v1 = 1.f - (f32)(j + 1) / N;
        P ul{ax, ay, u0, v0}, ll{ax, ay - cell, u0, v1};
        P ur{ax + cell, ay, u1, v0}, lr{ax + cell, ay - cell, u1, v1};
        const P *tri[2][3] = {{&ll, &lr, &ur}, {&ll, &ur, &ul}};
        for (auto &t : tri) {
          SV sv[3];
          for (int k = 0; k < 3; k++) {
            double p[4] = {t[k]->x, t[k]->y, 0, 1}, e[4];
            for (int r = 0; r < 4; r++)
              e[r] = mv.m[r][0] * p[0] + mv.m[r][1] * p[1] + mv.m[r][2] * p[2] + mv.m[r][3] * p[3];
            double df = 375.0 * sample_h(hm, t[k]->u, t[k]->v);
            for (int r = 0; r < 3; r++) e[r] += nv[r] * df;
            double o[4];
            for (int r = 0; r < 4; r++)
              o[r] = proj.m[r][0] * e[0] + proj.m[r][1] * e[1] + proj.m[r][2] * e[2] +
                     proj.m[r][3] * e[3];
            sv[k] = {o[0], o[1], o[2], o[3]};
          }
          c.tris++;
          // clip against the near plane z >= -w (Sutherland-Hodgman), then fan
          std::vector<SV> poly(sv, sv + 3), out;
          bool any_out = false;
          for (int k = 0; k < 3; k++) if (sv[k].z < -sv[k].w) any_out = true;
          if (any_out) {
            for (size_t k = 0; k < poly.size(); k++) {
              SV a = poly[k], b = poly[(k + 1) % poly.size()];
              double da = a.z + a.w, db = b.z + b.w;
              if (da >= 0) out.push_back(a);
              if ((da >= 0) != (db >= 0)) {
                double s = da / (da - db);
                out.push_back({a.x + (b.x - a.x) * s, a.y + (b.y - a.y) * s,
                               a.z + (b.z - a.z) * s, a.w + (b.w - a.w) * s});
              }
            }
            poly = out;
          }
          for (size_t k = 1; k + 1 < poly.size(); k++) {
            SV tr[3] = {poly[0], poly[k], poly[k + 1]};
            raster(tr, depth, hits, c, q24);
          }
        }
      }
    printf("t=%.2fs triangles=%llu drawn=%llu culled=%llu rasterized=%llu passed=%llu "
           "covered=%llu far_clipped=%llu overdraw=%.4f passed_per_covered=%.4f\n",
           secs, (unsigned long long)c.tris, (unsigned long long)c.drawn,
           (unsigned long long)c.culled, (unsigned long long)c.rast,
           (unsigned long long)c.passed, (unsigned long long)c.covered,
           (unsigned long long)c.clipped_z, c.rast / (double)c.covered,
           c.passed / (double)c.covered);
    fflush(stdout);
  }
  return 0;
}
