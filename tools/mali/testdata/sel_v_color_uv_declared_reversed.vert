/* Varyings (L3 reference): color_uv with the vec2 declared first: the layout still puts the vec4 first (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos;
attribute vec4 a_color;
attribute vec2 a_uv;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
    v_uv = a_uv;
}
