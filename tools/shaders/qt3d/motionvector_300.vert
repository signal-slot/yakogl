#version 300 es

struct buf
{
    mat4 mvp;
    mat4 prevMVP;
    mat4 instanceLocal;
    mat4 instanceGlobal;
    mat4 prevInstanceLocal;
    mat4 prevInstanceGlobal;
    vec4 currentAndLastJitter;
    float velocityAmount;
    int morphTargetCount;
    vec2 padding;
};

uniform buf ubuf;

layout(location = 0) in vec3 attr_pos;
out vec4 vPosition;
out vec4 vPrevPosition;

void main()
{
    vec4 vp = vec4(attr_pos, 1.0);
    vec4 vpPre = vec4(attr_pos, 1.0);
    vec4 _41 = ubuf.mvp * vp;
    vPosition = _41;
    gl_Position = _41;
    vPrevPosition = ubuf.prevMVP * vpPre;
}

