#version 100
/* float -> bool on data (v3d back end: F2b as FMOV.pushz, x != 0.0): a
 * scalar that is zero on part of the triangle, a bvec2 conversion, and
 * -0.0, which is zero and so false. dEQP's swizzles and conversions groups
 * are mostly this shape. */
precision mediump float;
varying vec4 v_color;
void main() {
    float z = floor(2.0 * v_color.r);           /* 0.0 or 1.0 */
    bvec2 bv = bvec2(vec2(floor(2.0 * v_color.g), floor(2.0 * v_color.b)));
    bool nz = bool(-z * 0.0);                   /* -0.0 or 0.0: false */
    gl_FragColor = vec4(bool(z) ? 1.0 : 0.0, bv.x ? 1.0 : 0.25, bv.y ? 1.0 : 0.5, nz ? 0.0 : 1.0);
}
