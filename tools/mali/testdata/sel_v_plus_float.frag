/* Varyings (L3 reference): v_c + v_f (a lane alone, .h00, in a sum) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; varying vec4 v_c; varying float v_f;
void main() { gl_FragColor = v_c + v_f; }
