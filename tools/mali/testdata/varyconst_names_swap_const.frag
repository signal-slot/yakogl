/* Issue #4510: the FS of varyconst_names_swap_const; it reads `b`, its location 0 and the vertex shader's location 1: no varying is left to read (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 b; varying vec4 a; void main() { gl_FragColor = b; }
