#version 100
/* #32 plan #367 results-row: terrain's tilt-shift: a step computed from the varying, 5 vertical taps (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_MULTI, desktop
 * blur and terrain's look-ups beside another at a mediump computed coordinate). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
const float StepY = 0.004;
const float Tilt = 0.5;
void main() {
    vec2 tc = v_uv;
    float s = StepY * abs(Tilt - tc.y) / abs(1.0 - Tilt);
    vec4 r =
        texture2D(u_s0, tc + vec2(0.0, -2.0 * s)) * 0.1875 +
        texture2D(u_s0, tc + vec2(0.0, -1.0 * s)) * 0.203125 +
        texture2D(u_s0, tc + vec2(0.0, 0.0 * s)) * 0.21875 +
        texture2D(u_s0, tc + vec2(0.0, 1.0 * s)) * 0.203125 +
        texture2D(u_s0, tc + vec2(0.0, 2.0 * s)) * 0.1875 +
        0.0;
    gl_FragColor = vec4(r.xyz, 1.0);
}
