/* A multiply by a constant: Mesa's algebraic rules on it (m2). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.xy * 2.0, 0.0, 1.0); }
