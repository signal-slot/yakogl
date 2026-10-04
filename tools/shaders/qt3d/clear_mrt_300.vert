#version 300 es

layout(location = 0) in vec3 attr_pos;

void main()
{
    vec3 p = attr_pos;
    gl_Position = vec4(p, 1.0);
}

