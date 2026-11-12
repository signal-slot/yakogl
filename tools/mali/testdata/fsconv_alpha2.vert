/* Three vec2 varyings and a float (the vertex shader of fsconv_alpha2). */
attribute vec4 a_pos; attribute vec4 a_d; attribute vec4 a_e;
varying vec2 p;
varying vec2 q;
varying vec2 r;
varying float f;
void main() {
  gl_Position = a_pos;
  p = a_e.xy;
  q = a_e.zw;
  r = a_d.zw;
  f = a_d.x;
}
