/* One vec4 varying (the vertex shader of fsload_cancel3). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec4 a;
void main() {
  gl_Position = a_pos;
  a = a_a;
}
