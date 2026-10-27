/* #1385 (b): the smallest look-up at a constant coordinate, the form of
 * gl_test cases 122 and 197 alone (tools/mali/drm_shim_ref.sh run, Mesa
 * 26.2.1; the pair's vertex shader is tri.vert). */
precision mediump float;
uniform sampler2D s;
void main() { gl_FragColor = texture2D(s, vec2(0.5)); }
