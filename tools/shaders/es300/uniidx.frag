#version 300 es
precision highp float;
precision highp int;
uniform vec4 k[16];
uniform mat4 m[4];
uniform int n;
uniform int j;
in vec4 v_t;
out vec4 colour;
void main()
{
    vec4 c = vec4(0.0);
    for (int i = 0; i < 16; i++)
    {
        if (i >= n)
            break;
        c += k[i] * v_t;
    }
    colour = m[j & 3] * c + k[(j + 5) & 15];
}
