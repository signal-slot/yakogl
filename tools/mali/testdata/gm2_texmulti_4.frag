#version 100
/* #32 plan #367 results-row: a 7-tap horizontal kernel stepped by a mediump uniform (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_MULTI, desktop
 * blur and terrain's look-ups beside another at a mediump computed coordinate). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
uniform float u_step;
void main() {
    vec4 r =
        texture2D(u_s0, v_uv + vec2(-3.0 * u_step, 0.0)) * 0.03125 +
        texture2D(u_s0, v_uv + vec2(-2.0 * u_step, 0.0)) * 0.109375 +
        texture2D(u_s0, v_uv + vec2(-1.0 * u_step, 0.0)) * 0.21875 +
        texture2D(u_s0, v_uv) * 0.28125 +
        texture2D(u_s0, v_uv + vec2(1.0 * u_step, 0.0)) * 0.21875 +
        texture2D(u_s0, v_uv + vec2(2.0 * u_step, 0.0)) * 0.109375 +
        texture2D(u_s0, v_uv + vec2(3.0 * u_step, 0.0)) * 0.03125;
    gl_FragColor = vec4(r.xyz, 1.0);
}
