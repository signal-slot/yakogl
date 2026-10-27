/* Varyings (L3 reference): the FS of sel_v_v4v4v2 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float;
varying vec4 v_c;
varying vec4 v_d;
varying vec2 v_t;
void main() { gl_FragColor = v_c + v_d * vec4(v_t, 1.0, 1.0); }
