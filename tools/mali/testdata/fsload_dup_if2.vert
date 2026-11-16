/* p written before an if, q equal on both sides (the vertex shader of fsload_dup_if2). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b;
varying vec2 p; varying vec2 q;
void main() {
  gl_Position = a_pos;
  p = a_a.xy; if (a_pos.x > 0.0) q = a_a.xy; else q = a_a.xy;
}
