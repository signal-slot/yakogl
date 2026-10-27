/* Varyings (L3 reference): three varyings: two vec4s and a vec2 at 32 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos;
attribute vec4 a_c;
attribute vec4 a_d;
attribute vec2 a_t;
varying vec4 v_c;
varying vec4 v_d;
varying vec2 v_t;
void main() {
    gl_Position = a_pos;
    v_c = a_c;
    v_d = a_d;
    v_t = a_t;
}
