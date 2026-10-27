/* Varyings (L3 reference): the VS of sel_v_fma_plus_vec3 (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_c; varying vec4 v_c; attribute vec4 a_d; varying vec4 v_d; attribute vec3 a_n; varying vec3 v_n;
void main() { gl_Position = a_pos; v_c = a_c; v_d = a_d; v_n = a_n; }
