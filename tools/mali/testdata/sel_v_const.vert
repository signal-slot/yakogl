/* Varyings (L3 reference): constant lanes (Mesa's link moves them into the fragment shader): refused (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec2 a_t; varying vec4 v_c;
void main() { gl_Position = a_pos; v_c = vec4(a_t, 0.0, 1.0); }
