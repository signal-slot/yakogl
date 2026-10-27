/* Varyings (L3 reference): v_c * v_d + u_m (a uniform word as the FMA's addend) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; uniform vec4 u_m; varying vec4 v_c; varying vec4 v_d;
void main() { gl_FragColor = v_c * v_d + u_m; }
