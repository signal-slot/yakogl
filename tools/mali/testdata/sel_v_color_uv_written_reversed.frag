/* Varyings (L3 reference): the FS of sel_v_color_uv_written_reversed (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float;
varying vec2 v_uv;
varying vec4 v_color;
void main() { gl_FragColor = v_color * vec4(v_uv, 1.0, 1.0); }
