/* Varyings (L3 reference): the FS of sel_v_v4_float (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float;
varying vec4 v_c;
varying float v_f;
void main() { gl_FragColor = v_c * v_f; }
