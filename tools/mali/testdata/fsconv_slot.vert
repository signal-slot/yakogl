/* Two vec3 varyings (the vertex shader of fsconv_slot). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec3 c;
varying vec3 d;
void main() {
  gl_Position = a_pos;
  c = a_a.xyz;
  d = a_b.xyz;
}
