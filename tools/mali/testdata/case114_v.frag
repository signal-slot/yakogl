#version 300 es
// gl_test case "cubelod": the vertex stage's sample, unchanged.
precision highp float;
in vec4 v_col;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = v_col;
}
