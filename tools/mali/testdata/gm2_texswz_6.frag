#version 100
/* #32 plan #367 lower-tex-swizzle: a grey of a texture result's rgb beside its own .a, mediump (original reduction
 * of terrain fs d697's shape) (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_SWIZZLE, see
 * docs/notes/mali-tex-swizzle.md). */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_s0;
void main() {
    vec4 c = texture2D(u_s0, v_uv);
    float g = dot(c.rgb, vec3(0.3, 0.59, 0.11));
    gl_FragColor = vec4(g, g, g, c.a);
}
