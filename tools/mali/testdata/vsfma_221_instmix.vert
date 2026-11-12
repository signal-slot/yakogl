#version 300 es
precision highp float;
layout(location = 0) in vec4 pos;
layout(location = 1) in float a_d1;
layout(location = 2) in float a_d3;
out vec4 v_col;
void main()
{
    float id = float(gl_InstanceID);
    vec2 cell = vec2(mod(id, 2.0) - 0.5, floor(id * 0.5) - 0.5);
    gl_Position = vec4(pos.xy * 0.5 + cell, 0.0, 1.0);
    v_col = vec4(a_d1, a_d3, id * 0.25, 1.0);
}
