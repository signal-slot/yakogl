/* Issue #4510: the FS of varyconst_names_undeclared_mixed; no constant lane reaches it (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }
