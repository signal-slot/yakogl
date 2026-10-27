/* Varyings (L3 reference): the VS of sel_v_plus_float (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_c; varying vec4 v_c; attribute float a_f; varying float v_f;
void main() { gl_Position = a_pos; v_c = a_c; v_f = a_f; }
