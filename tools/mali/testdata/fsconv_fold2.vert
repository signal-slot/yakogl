/* A vec3 and two vec2 varyings (the vertex shader of fsconv_fold2 and fsconv_fold3). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec3 c;
varying vec2 p;
varying vec2 q;
void main() {
  gl_Position = a_pos;
  c = a_a.xyz;
  p = a_b.xy;
  q = a_b.zw;
}
