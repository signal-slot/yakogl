/* Varyings (L3 reference): a vec4 and a vec3 varying, the vec3 at 16 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos;
attribute vec4 a_c;
attribute vec3 a_n;
varying vec4 v_c;
varying vec3 v_n;
void main() {
    gl_Position = a_pos;
    v_c = a_c;
    v_n = a_n;
}
