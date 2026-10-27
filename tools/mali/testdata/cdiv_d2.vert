/* A division: a * rcp(b) (d2). */
attribute vec4 a_pos; uniform vec2 u;
void main() { gl_Position = vec4(2.0 * a_pos.x / u.x - 1.0, 1.0 - 2.0 * a_pos.y / u.y, 0.0, 1.0); }
