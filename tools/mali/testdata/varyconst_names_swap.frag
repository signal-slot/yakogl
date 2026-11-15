/* Issue #4510: the FS of varyconst_names_swap; it reads `a`, the varying that is not constant (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 b; varying vec4 a; void main() { gl_FragColor = a; }
