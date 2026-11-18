#version 100
/* #32 plan #367 results-row: two look-ups at the varying plus constant offsets, weighted (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_MULTI, desktop
 * blur and terrain's look-ups beside another at a mediump computed coordinate). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
void main() {
    vec4 r = texture2D(u_s0, v_uv + vec2(-0.25, 0.0)) * 0.375 +
             texture2D(u_s0, v_uv + vec2(0.25, 0.0)) * 0.625;
    gl_FragColor = vec4(r.xyz, 1.0);
}
