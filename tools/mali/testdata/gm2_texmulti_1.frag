#version 100
/* #32 plan #367 results-row: a look-up at the varying beside one at the varying plus a constant (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_MULTI, desktop
 * blur and terrain's look-ups beside another at a mediump computed coordinate). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
void main() {
    gl_FragColor = texture2D(u_s0, v_uv) + texture2D(u_s0, v_uv + vec2(0.0, 0.25));
}
