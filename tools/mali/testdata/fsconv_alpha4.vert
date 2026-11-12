/* A float, a vec3, a vec2 and a vec3 (the vertex shader of fsconv_alpha4). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b; attribute vec4 a_d; attribute vec4 a_e;
varying float f;
varying vec3 c;
varying vec2 p;
varying vec3 d;
void main() {
  gl_Position = a_pos;
  f = a_d.x;
  c = a_a.xyz;
  p = a_e.xy;
  d = a_b.xyz;
}
