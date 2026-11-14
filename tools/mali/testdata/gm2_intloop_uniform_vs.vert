#version 100
/* The same vertex loop with a uniform int trip count, as the benchmark
 * writes it (ES 1.00 asks for a constant bound; Mesa accepts the uniform). */
attribute vec3 a_pos;
uniform mat4 u_mvp;
uniform int u_loops;
varying vec4 v_dummy;
void main()
{
    v_dummy = vec4(1.0);
    float d = fract(a_pos.x);
    for (int i = 0; i < u_loops; i++)
        d = fract(3.0 * d);
    d = fract(3.0 * d);
    vec4 pos = vec4(a_pos.x, a_pos.y + 0.1 * d * fract(a_pos.x), a_pos.z, 1.0);
    gl_Position = u_mvp * pos;
}
