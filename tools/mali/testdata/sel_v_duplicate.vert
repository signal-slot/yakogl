/* Varyings (L3 reference): two varyings of equal value (Mesa's link merges them): refused (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_c; varying vec4 v_a; varying vec4 v_b;
void main() { gl_Position = a_pos; v_a = a_c; v_b = a_c; }
