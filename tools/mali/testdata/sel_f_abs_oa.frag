/* A fragment fabs (1.0 - abs(v_color * 2.0 - 1.0)), drm-shim reference (Mesa 26.2.1's GL driver, the GLSL path, tools/mali/drm_shim_ref.sh run; with gl_test's passthrough.vert). */
precision mediump float; varying vec4 v_color;
void main() { gl_FragColor = 1.0 - abs(v_color * 2.0 - 1.0); }
