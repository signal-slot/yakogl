#version 100
/* Three conditional steps in a vertex shader, no loop. */
attribute vec3 a_pos;
uniform mat4 u_mvp;
varying vec4 v_dummy;
void main()
{
    v_dummy = vec4(1.0);
    float d = fract(a_pos.x);
    d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    if (d >= 0.5)
        d = fract(2.0 * d);
    else
        d = fract(3.0 * d);
    vec4 pos = vec4(a_pos.x, a_pos.y + 0.1 * d * fract(a_pos.x), a_pos.z, 1.0);
    gl_Position = u_mvp * pos;
}
