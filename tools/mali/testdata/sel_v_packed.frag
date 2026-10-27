/* Varyings (L3 reference): the FS of sel_v_packed: one LD_VAR of the packed slot (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float;
varying vec2 v_a;
varying vec2 v_b;
void main() { gl_FragColor = vec4(v_a, v_b); }
