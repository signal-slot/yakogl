/* An if whose sides both store a_a.xy to p and q (the vertex shader of fsload_dup_if). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec2 p; varying vec2 q;
void main() {
  gl_Position = a_pos;
  if (a_pos.x > 0.0) { p = a_a.xy; q = a_a.xy; } else { p = a_a.xy; q = a_a.xy; }
}
