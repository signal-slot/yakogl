#version 300 es
/* gl_test 196: trunc() on both signs, and integer shifts (arithmetic on a
 * negative int, logical on a uint), of values a varying carries exactly. */
precision highp float;
precision highp int;
in vec4 v;
out vec4 o_color;
void main() {
    int i = int(v.y);
    uint u = uint(v.z);
    o_color = vec4((trunc(v.x) + 2.0) * 0.125, (trunc(-v.x) + 4.0) * 0.125,
                   float((i >> 1) - (i << 3)) / 64.0, float(u >> 2u) / 64.0);
}
