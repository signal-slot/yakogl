#version 100
#extension GL_OES_standard_derivatives : enable
/* Derivatives read back: gl_test case "deriv".
 *
 * Red and green are 0.5 plus a derivative of the varying scaled to 0.25, so
 * the right value reads 0.75, a wrong sign 0.25 and no derivative 0.5:
 * dFdx(v.x) = dFdy(v.y) = 1/32 (deriv.vert's step on a 64x64 target), with
 * GL's y running up, in a window and in an FBO alike. Blue is 0.5 plus an
 * eighth of dFdx(gl_FragCoord.x) + dFdy(gl_FragCoord.y), both 1: 0.75, and
 * 0.5 when dFdy's sign is wrong. Alpha is 1 (a window may keep no alpha). */
precision mediump float;
varying vec2 v;
void main() {
    gl_FragColor = vec4(0.5 + 8.0 * dFdx(v.x), 0.5 + 8.0 * dFdy(v.y),
                        0.5 + 0.125 * (dFdx(gl_FragCoord.x) + dFdy(gl_FragCoord.y)), 1.0);
}
