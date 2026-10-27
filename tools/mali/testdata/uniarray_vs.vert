#version 100
/* #1385 (b), plan #153 uniarray: a float uniform array read at a run-time
 * index in the vertex stage (case 108 B's read, moved to the vertex
 * shader; tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; the pair's fragment
 * shader is uniarray_vs.frag). */
attribute vec4 a_pos;
uniform int ui_two;
uniform float u_arr[4];
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = vec4(u_arr[ui_two], 0.0, 0.0, 1.0);
}
