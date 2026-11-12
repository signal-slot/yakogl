/* A vec3 and a vec2 varying (the vertex shader of fsconv_disc2 and fsconv_discpc). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_e;
varying vec3 c;
varying vec2 p;
void main() {
  gl_Position = a_pos;
  c = a_a.xyz;
  p = a_e.xy;
}
