/* Three vec3 varyings and a float (the vertex shader of fsconv_alpha3). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b; attribute vec4 a_c; attribute vec4 a_d;
varying vec3 c;
varying vec3 d;
varying vec3 e;
varying float f;
void main() {
  gl_Position = a_pos;
  c = a_a.xyz;
  d = a_b.xyz;
  e = a_c.xyz;
  f = a_d.x;
}
