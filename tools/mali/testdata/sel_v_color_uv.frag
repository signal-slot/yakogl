/* Varyings (L3 reference): gl_test's color_uv.frag (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1 on drm-shim). */
#version 100
/* Two-varying FS: colour modulated by uv. ES 1.00 twin of gl_test_fs_src. */
precision mediump float;
varying vec4 v_color;
varying vec2 v_uv;
void main() { gl_FragColor = v_color * vec4(v_uv, 1.0, 1.0); }
