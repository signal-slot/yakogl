// gl_test 117 uniidxf: a vertex shader with a uniform and NO run-time
// index, under 108 D's fragment shader (uniidx_d.frag): with 116 it tells
// "the vertex stage has uniforms" (PS_UNIFORM_BASE > 0) from "the vertex
// stage indexes".
attribute vec4 a_pos;
uniform vec4 u_k;
varying vec4 v_color;
void main()
{
    gl_Position = a_pos;
    v_color = u_k;
}
