#version 100
/* gl_test 222 dtpu: the sampled depth .r rebuilt as its 24-bit word, k = round(r * (2^24 - 1)),
 * written as (k >> 16, k >> 8 & 255, k & 255, 255) / 255 so the read-back's R * 65536 + G * 256 + B
 * is the word within 2 (two fp32 roundings near 2^24). The same shader decodes the CPU resolve and
 * the texture unit's D24X8 view, so equal pixels mean equal sampled values (docs/notes/gc-depth-sample.md). */
precision highp float;
uniform sampler2D u_tex;
varying vec2 v_uv;
void main() {
    float k = floor(texture2D(u_tex, v_uv).r * 16777215.0 + 0.5);
    float hi = floor(k / 65536.0);
    float mid = floor((k - hi * 65536.0) / 256.0);
    float lo = k - hi * 65536.0 - mid * 256.0;
    gl_FragColor = vec4(hi, mid, lo, 255.0) / 255.0;
}
