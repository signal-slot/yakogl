/* Issue #4510: a constant varying `b` at the vertex shader's location 1. The fragment shader declares `b` first, so its location 1 is `a` (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 p; varying vec4 a; varying vec4 b;
void main() { gl_Position = p; a = p; b = vec4(0.25, 0.5, 0.75, 1.0); }
