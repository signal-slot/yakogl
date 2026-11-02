#version 100
/* A uniform branch read back: gl_test case "unibranch".
 *
 * The `if` depends on a uniform only and each arm holds far more than 24
 * scalar operations, so the V3D back end branches round the untaken arm
 * (Mesa's rule, docs/online-shaderc/v3d-backend.md "Uniform branches")
 * instead of computing both and selecting. The case draws once with u.x = 1
 * (the then arm) and once with u.x = 0 (the else arm); u.yzw are fixed. Each
 * arm's colour is computed on the CPU with the same formula, and the two
 * colours differ by far more than the tolerance, so an arm taken wrongly, a
 * merge lost or a uniform read from the skipped arm shows. Alpha is 1 (a
 * window may keep no alpha). */
precision mediump float;
uniform vec4 u;
varying vec2 v;
void main() {
    vec4 c;
    if (u.x > 0.5) {
        vec4 a = vec4(u.y, u.z, u.w, 0.5);
        vec4 b = a * a.yzwx + a.zwxy * a.wxyz;
        c = b * 0.5 - a * 0.25 + a.wzyx * 0.125;
        c = clamp(c * c.yzwx + a * 0.5, 0.0, 1.0);
    } else {
        vec4 a = vec4(u.w, u.z, u.y, 0.25);
        vec4 b = a * a + a.yzwx * 0.5;
        c = b * a.zwxy - a * 0.125 + a.wxyz * 0.25;
        c = clamp(c + c.yzwx * 0.25, 0.0, 1.0);
    }
    gl_FragColor = vec4(c.rgb, 1.0);
}
