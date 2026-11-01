#version 300 es
layout(location = 0) in vec2 pos;
out vec2 v_xy;
void main()
{
    v_xy = pos;
    gl_Position = vec4(pos, 0.0, 1.0);
}
