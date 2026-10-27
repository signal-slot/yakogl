/* A multiply by a constant: Mesa's algebraic rules on it (m3mix). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.x * 3.0, a_pos.y * 0.75, 0.0, 1.0); }
