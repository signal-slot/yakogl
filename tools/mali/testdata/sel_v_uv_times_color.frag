/* Varyings (L3 reference): color_uv.frag with the operands swapped: the FMA takes the vec2 first (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
precision mediump float;
varying vec4 v_color;
varying vec2 v_uv;
void main() { gl_FragColor = vec4(v_uv, 1.0, 1.0) * v_color; }
