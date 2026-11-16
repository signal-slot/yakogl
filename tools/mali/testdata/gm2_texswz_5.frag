#version 100
/* #32 plan #367 ref-tex-swizzle: two texture results mixed by 1.0 minus a third (the blend shape of the terrain shader), mediump (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_SWIZZLE, see
 * docs/notes/mali-texture-refusals.md). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
uniform sampler2D u_s1;
uniform sampler2D u_s2;
void main() {
    vec4 a = texture2D(u_s0, v_uv);
    vec4 b = texture2D(u_s1, v_uv);
    gl_FragColor = mix(a, b, 1.0 - texture2D(u_s2, v_uv));
}
