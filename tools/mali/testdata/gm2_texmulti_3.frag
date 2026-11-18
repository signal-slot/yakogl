#version 100
/* #32 plan #367 results-row: a 5-tap vertical kernel at constant offsets and weights, blur's sum (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_MULTI, desktop
 * blur and terrain's look-ups beside another at a mediump computed coordinate). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
const float StepY = 0.0025;
const float W2 = 0.0625;
const float W1 = 0.25;
const float W0 = 0.375;
void main() {
    vec4 r =
        texture2D(u_s0, v_uv + vec2(0.0, -2.0 * StepY)) * W2 +
        texture2D(u_s0, v_uv + vec2(0.0, -1.0 * StepY)) * W1 +
        texture2D(u_s0, v_uv + vec2(0.0, 0.0 * StepY)) * W0 +
        texture2D(u_s0, v_uv + vec2(0.0, 1.0 * StepY)) * W1 +
        texture2D(u_s0, v_uv + vec2(0.0, 2.0 * StepY)) * W2 +
        0.0;
    gl_FragColor = vec4(r.xyz, 1.0);
}
