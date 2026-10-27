/* Varyings (L3 reference): v_c * v_f + v_d (a lane alone in an FMA) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v_c; varying vec4 v_d; varying float v_f;
void main() { gl_FragColor = v_c * v_f + v_d; }
