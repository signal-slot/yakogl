#version 100
/* glmark2 22c527cb conditionals:vertex-steps=5:fragment-steps=0, vertex
 * shader a21c916f: five two-armed `if`s in a row on a float compare with a
 * constant, each arm a fract of a constant times the value, in a vertex
 * shader; the value then moves the position, which a matrix transforms. */
attribute vec3 a_pos;
uniform mat4 u_mvp;
void main() {
    float t = fract(a_pos.x);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    if (t >= 0.5) t = fract(2.0 * t); else t = fract(3.0 * t);
    gl_Position = u_mvp * vec4(a_pos.x, a_pos.y + t, a_pos.z, 1.0);
}
