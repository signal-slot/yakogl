/* A fragment fabs (abs(v_color) * 0.5), drm-shim reference (Mesa 26.2.1's GL driver, the GLSL path, tools/mali/drm_shim_ref.sh run; with gl_test's passthrough.vert). */
precision mediump float; varying vec4 v_color;
void main() { gl_FragColor = abs(v_color) * 0.5; }
