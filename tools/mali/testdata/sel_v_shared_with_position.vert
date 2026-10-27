/* Varyings (L3 reference): the position's value written as a varying too (the varying program computes it again) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u_k; varying vec4 v_p;
void main() { vec4 p = a_pos * u_k; gl_Position = p; v_p = p; }
