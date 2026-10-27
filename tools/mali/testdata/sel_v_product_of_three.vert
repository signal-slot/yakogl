/* Varyings (L3 reference): the VS of sel_v_product_of_three (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_c; varying vec4 v_c; attribute vec4 a_d; varying vec4 v_d; attribute vec4 a_e; varying vec4 v_e;
void main() { gl_Position = a_pos; v_c = a_c; v_d = a_d; v_e = a_e; }
