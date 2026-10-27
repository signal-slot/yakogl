#version 300 es
precision mediump float;
in vec4 v_col;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = v_col;
}
