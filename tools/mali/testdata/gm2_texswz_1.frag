#version 100
/* #32 plan #367 ref-tex-swizzle: a texture result's .r alone, mediump (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_SWIZZLE, see
 * docs/notes/mali-texture-refusals.md). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
void main() {
    gl_FragColor = vec4(texture2D(u_s0, v_uv).r);
}
