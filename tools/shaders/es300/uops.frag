#version 300 es
precision highp float;
precision highp int;
uniform uvec2 u;
uniform ivec2 i;
uniform float f;
out vec4 colour;
void main() {
    uint d = u.x / u.y;
    uint m = u.x % u.y;
    uint sh = u.x >> 3u;
    uint umn = min(u.x, u.y);
    uint umx = max(u.x, u.y);
    int mn = min(i.x, i.y);
    int mx = max(i.x, i.y);
    uint nt = ~u.x;
    uint fu = uint(f);
    uint mix1 = d ^ (m << 8u) ^ (sh << 16u) ^ umn ^ (umx << 5u) ^ nt ^ (fu << 3u) ^ uint(mn) ^ (uint(mx) << 11u);
    colour = vec4(float(mix1 & 255u) / 255.0, float((mix1 >> 8u) & 255u) / 255.0,
                  (u.x < u.y ? 0.25 : 0.0) + (u.x <= u.y ? 0.5 : 0.0), fract(float(u.x) / 65536.0));
}
