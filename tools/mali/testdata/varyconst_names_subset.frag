/* Issue #4510: the FS of varyconst_names_subset; its only varying `a` is its location 0 and the vertex shader's location 1 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 a; void main() { gl_FragColor = a; }
