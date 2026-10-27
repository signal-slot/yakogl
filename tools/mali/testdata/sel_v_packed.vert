/* Varyings (L3 reference): two vec2 varyings, which the GLSL link packs into one slot: refused (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos;
attribute vec2 a_a;
attribute vec2 a_b;
varying vec2 v_a;
varying vec2 v_b;
void main() {
    gl_Position = a_pos;
    v_a = a_a;
    v_b = a_b;
}
