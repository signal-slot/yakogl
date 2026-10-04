#version 300 es
precision highp float;
precision highp int;
in vec4 v_col;
uniform vec4 cols[8];
uniform int idx;
uniform int n;
layout(location = 0) out vec4 fragColor;
void main()
{
    vec4 c = vec4(0.0);
    for (int i = 0; i < n; ++i)
        c += cols[idx + i];
    fragColor = c;
}
