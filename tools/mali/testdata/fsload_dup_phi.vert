/* An if whose sides store a_a.xy or a_b.xy to both p and q (the vertex shader of fsload_dup_phi). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec2 p; varying vec2 q;
void main() {
  gl_Position = a_pos;
  if (a_pos.x > 0.0) { p = a_a.xy; q = a_a.xy; } else { p = a_b.xy; q = a_b.xy; }
}
