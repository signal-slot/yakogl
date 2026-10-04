#version 300 es

struct buf
{
    vec4 aoProperties;
    vec4 aoProperties2;
    vec4 aoScreenConst;
    vec4 uvToEyeConst;
    vec2 cameraProperties;
};

uniform buf ubuf;

layout(location = 0) in vec3 attr_pos;

void main()
{
    gl_Position = vec4(attr_pos.xy, 0.5, 1.0);
}

