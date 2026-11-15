/* Issue #4510: the FS of varyconst_names_undeclared; `w` is its location 0, where the vertex shader holds the constant `v` (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision highp float;
varying float w;
void main() { gl_FragColor = vec4(w, 0.0, 0.0, 1.0); }
