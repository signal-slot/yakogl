#version 100
/* glmark2 22c527cb function (both scenes, vertex-steps 1, complexity low),
 * vertex shader 01626306: one call of a float -> float function whose body
 * reassigns its parameter and returns it, in a vertex shader. */
attribute vec3 a_pos;
uniform mat4 u_mvp;
float step3(float x) {
    x = fract(3.0 * x);
    return x;
}
void main() {
    float t = fract(a_pos.x);
    t = step3(t);
    gl_Position = u_mvp * vec4(a_pos.x, a_pos.y + t, a_pos.z, 1.0);
}
