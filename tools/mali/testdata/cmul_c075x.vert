/* A multiply by a constant: Mesa's algebraic rules on it (c075x). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(0.75 * a_pos.xy, 0.0, 1.0); }
