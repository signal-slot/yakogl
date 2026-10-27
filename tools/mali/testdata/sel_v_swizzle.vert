/* Varyings (L3 reference): an attribute's lanes swizzled (a_c.yx) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_c; varying vec2 v_s;
void main() { gl_Position = a_pos; v_s = a_c.yx; }
