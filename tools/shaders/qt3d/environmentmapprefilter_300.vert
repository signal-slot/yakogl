#version 300 es

struct buf
{
    mat4 projection;
    mat4 view;
};

uniform buf ubuf;

out vec3 localPos;
layout(location = 0) in vec3 aPos;

void main()
{
    localPos = aPos;
    mat3 _32 = mat3(ubuf.view[0].xyz, ubuf.view[1].xyz, ubuf.view[2].xyz);
    mat4 rotView = mat4(vec4(_32[0].x, _32[0].y, _32[0].z, 0.0), vec4(_32[1].x, _32[1].y, _32[1].z, 0.0), vec4(_32[2].x, _32[2].y, _32[2].z, 0.0), vec4(0.0, 0.0, 0.0, 1.0));
    vec4 clipPos = (ubuf.projection * rotView) * vec4(localPos, 1.0);
    gl_Position = clipPos.xyww;
}

