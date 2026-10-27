#version 100
/* #1385 (b): gl_test case 197's fragment shader,
 * tools/shaders/gltest/vtl_fs.frag verbatim: a look-up at the constant
 * coordinate vec2(0.5, 0.5) mixed with a varying by a float uniform; the
 * pair's vertex shader is tex_vs_lod.vert (case 197's vtxgrid_vs.vert)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1, --uniform-values). */
precision mediump float;
uniform sampler2D u_ftex;
uniform float u_pick;
varying vec4 v_color;
void main() { gl_FragColor = mix(v_color, texture2D(u_ftex, vec2(0.5, 0.5)), u_pick); }
