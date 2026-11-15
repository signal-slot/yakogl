/* Issue #4510: the VS of varyconst_names_swap with a fragment shader that reads the constant varying (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute vec4 p; varying vec4 a; varying vec4 b;
void main() { gl_Position = p; a = p; b = vec4(0.25, 0.5, 0.75, 1.0); }
