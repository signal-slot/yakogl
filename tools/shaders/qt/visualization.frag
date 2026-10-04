#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    mat4 rotation;
    vec4 color;
    float pattern;
    int projection;
};

uniform buf ubuf;

varying vec2 pos;

void main()
{
    vec4 c = ubuf.color;
    vec4 _44 = c;
    vec3 _47 = _44.xyz + vec3((pow(max(sin(pos.x + pos.y), 0.0), 2.0) * ubuf.pattern) * 0.25);
    c.x = _47.x;
    c.y = _47.y;
    c.z = _47.z;
    gl_FragData[0] = c;
}

