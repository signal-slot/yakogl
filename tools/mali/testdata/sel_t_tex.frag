/* Textures (L3 reference): texture2D(sampler2D, vec2 varying) as the colour, gl_test case 2's shader (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec2 v_uv; uniform sampler2D u_tex; void main() { gl_FragColor = texture2D(u_tex, v_uv); }
