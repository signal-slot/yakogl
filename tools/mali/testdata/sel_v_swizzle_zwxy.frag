/* Varyings (L3 reference): v_c.zwxy + v_d: a converted half read at the other pair (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v_c; varying vec4 v_d; void main() { gl_FragColor = v_c.zwxy + v_d; }
