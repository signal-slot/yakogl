#version 100
/* #32 plan #367 results-row: terrain's tilt-shift at tilt 0.0: 3 horizontal taps, the centre tap at the varying (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_MULTI, desktop
 * blur and terrain's look-ups beside another at a mediump computed coordinate). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
const float StepX = 0.00125;
const float Tilt = 0.0;
void main() {
    float s = StepX * abs(Tilt - v_uv.y) / abs(1.0 - Tilt);
    vec4 r =
        texture2D(u_s0, v_uv + vec2(-1.0 * s, 0.0)) * 0.25 +
        texture2D(u_s0, v_uv) * 0.5 +
        texture2D(u_s0, v_uv + vec2(1.0 * s, 0.0)) * 0.25;
    gl_FragColor = vec4(r.xyz, 1.0);
}
