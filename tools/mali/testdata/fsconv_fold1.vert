/* One vec4 varying (the vertex shader of fsconv_fold1). */
attribute vec4 a_pos; attribute vec4 a_a;
varying vec4 a;
void main() {
  gl_Position = a_pos;
  a = a_a;
}
