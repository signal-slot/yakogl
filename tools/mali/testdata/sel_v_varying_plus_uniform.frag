/* Varyings (L3 reference): v_c + u_m (FADD.v2f16 with a uniform word) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; uniform vec4 u_m; varying vec4 v_c; void main() { gl_FragColor = v_c + u_m; }
