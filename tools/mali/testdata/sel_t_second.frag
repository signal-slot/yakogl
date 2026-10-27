/* Textures (L3 reference): two samplers declared, only the second read, gl_test's compact.frag: Mesa numbers the samplers the program reads (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec2 v_uv; uniform sampler2D u_tex0; uniform sampler2D u_tex1; void main() { gl_FragColor = texture2D(u_tex1, v_uv); }
