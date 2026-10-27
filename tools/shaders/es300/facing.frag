#version 300 es
precision mediump float;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0);
}
