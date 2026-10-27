/* Varyings (L3 reference): a uniform alone (Mesa's link rewrites it as an exact ffma): refused (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u_k; varying vec4 v_c;
void main() { gl_Position = a_pos; v_c = u_k; }
