#version 300 es
// gl_test case 104 "divloop": s runs 0..1 left to right across the quad.
layout(location = 0) in vec4 pos;
out float v_s;
void main()
{
    v_s = pos.x * 0.5 + 0.5;
    gl_Position = pos;
}
