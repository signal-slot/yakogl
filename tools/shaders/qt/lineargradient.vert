#version 100

struct buf
{
    mat4 matrix;
    mat4 gradientMatrix;
    vec2 gradStart;
    vec2 gradEnd;
    float opacity;
};

uniform buf ubuf;

attribute vec4 vertexCoord;
varying float gradTabIndex;
attribute vec4 vertexColor;

void main()
{
    vec2 gradVertexCoord = (ubuf.gradientMatrix * vertexCoord).xy;
    vec2 gradVec = ubuf.gradEnd - ubuf.gradStart;
    gradTabIndex = dot(gradVec, gradVertexCoord - ubuf.gradStart) / ((gradVec.x * gradVec.x) + (gradVec.y * gradVec.y));
    gl_Position = ubuf.matrix * vertexCoord;
}

