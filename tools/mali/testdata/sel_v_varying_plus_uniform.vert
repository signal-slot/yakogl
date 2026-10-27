/* Varyings (L3 reference): the VS of sel_v_varying_plus_uniform (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_c; uniform vec4 u_k; varying vec4 v_c;
void main() { gl_Position = a_pos; v_c = a_c; }
