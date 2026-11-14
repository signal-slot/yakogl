#version 100
/* A vertex loop of a constant trip count: five steps of fract(3.0 * d) on
 * fract(position.x), then one more, moving the vertex's y. */
attribute vec3 a_pos;
uniform mat4 u_mvp;
varying vec4 v_dummy;
void main()
{
    v_dummy = vec4(1.0);
    float d = fract(a_pos.x);
    for (int i = 0; i < 5; i++)
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    vec4 pos = vec4(a_pos.x, a_pos.y + 0.1 * d * fract(a_pos.x), a_pos.z, 1.0);
    gl_Position = u_mvp * pos;
}
