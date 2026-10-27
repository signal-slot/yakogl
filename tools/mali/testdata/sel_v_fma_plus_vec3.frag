/* Varyings (L3 reference): v_c * v_d + vec4(v_n, 1.0) (MKVEC as the FMA's addend) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v_c; varying vec4 v_d; varying vec3 v_n;
void main() { gl_FragColor = v_c * v_d + vec4(v_n, 1.0); }
