/* Varyings (L3 reference): v_c * v_d * v_e (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v_c; varying vec4 v_d; varying vec4 v_e;
void main() { gl_FragColor = v_c * v_d * v_e; }
