#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    vec4 color;
};

uniform buf _13;

void main()
{
    gl_FragData[0] = _13.color;
}

