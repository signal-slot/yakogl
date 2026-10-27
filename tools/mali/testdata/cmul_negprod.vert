/* A product with a negated factor: fneg(fmul(a, b)) (negprod). */
attribute vec4 a_pos; uniform vec2 u;
void main() { gl_Position = vec4(-a_pos.x * u.x, a_pos.y, 0.0, 1.0); }
