/* Varyings (L3 reference): the FS of sel_v_duplicate (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v_a; varying vec4 v_b; void main() { gl_FragColor = v_a * v_b; }
