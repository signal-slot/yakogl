/* A multiply by a constant: Mesa's algebraic rules on it (w2). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.xyz, 2.0); }
