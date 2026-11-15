/* Issue #4510: a constant varying `v` the fragment shader does not declare, at the location its `w` has there (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
attribute highp vec4 a_position; attribute float a; varying float v; varying float w;
void main() { gl_Position = a_position; v = 0.5; w = a; }
