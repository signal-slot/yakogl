/* Issue #4510: constant lanes in a varying `w_k` the fragment shader does not declare, after the varying it reads (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_v; varying vec4 v; varying highp vec4 w_k;
void main() { gl_Position = a_pos; w_k = vec4(a_pos.x, 0.5, 0.25, 1.0); mediump float p = a_v.x * 2.0; v = a_v * p; }
