/* A vec3 and a vec2 varying (the vertex shader of fsload_bare2 and fsload_bare4). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec3 c; varying vec2 p;
void main() {
  gl_Position = a_pos;
  c = a_a.xyz; p = a_b.xy;
}
