#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 skyboxProperties;
    mat3 orientation;
    mat4 viewProjection;
    mat4 inverseProjection;
    mat3 viewMatrix;
};

uniform buf ubuf;

uniform highp samplerCube tex;

in vec3 tex_coords;
layout(location = 0) out vec4 fragOutput;

void main()
{
    vec4 color = texture(tex, tex_coords);
    fragOutput = color;
}

