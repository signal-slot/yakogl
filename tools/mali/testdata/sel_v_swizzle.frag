/* Varyings (L3 reference): the FS of sel_v_swizzle (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec2 v_s; void main() { gl_FragColor = vec4(v_s, v_s); }
