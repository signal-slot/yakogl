/* q is p swizzled (the vertex shader of fsload_dup_swz). */
attribute vec4 a_pos; attribute vec4 a_e;
varying vec2 p;
varying vec2 q;
void main() {
  gl_Position = a_pos;
  p = a_e.xy;
  q = p.yx;
}
