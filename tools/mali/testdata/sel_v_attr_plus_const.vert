/* Varyings (L3 reference): a vec2 attribute plus a constant (FADD_IMM) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec2 a_t; varying vec2 v_t;
void main() { gl_Position = a_pos; v_t = a_t + vec2(0.5, 0.25); }
