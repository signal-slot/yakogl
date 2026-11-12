/* One vec4 varying (the vertex shader of fsconv_discw, fsconv_disch and fsconv_discv). */
attribute vec4 a_pos; attribute vec4 a_f;
varying vec4 a;
void main() {
  gl_Position = a_pos;
  a = a_f;
}
