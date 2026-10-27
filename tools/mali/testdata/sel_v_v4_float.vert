/* Varyings (L3 reference): a vec4 and a float varying, the float at 16 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos;
attribute vec4 a_c;
attribute float a_f;
varying vec4 v_c;
varying float v_f;
void main() {
    gl_Position = a_pos;
    v_c = a_c;
    v_f = a_f;
}
