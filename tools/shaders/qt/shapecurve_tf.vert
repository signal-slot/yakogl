#version 100

struct buf
{
    mat4 qt_Matrix;
    float matrixScale;
    float opacity;
    float debug;
    float reserved3;
    mat4 gradientMatrix;
    vec2 boundsSize;
};

uniform buf ubuf;

attribute vec2 normalVector;
varying vec4 qt_TexCoord;
attribute vec4 vertexTexCoord;
attribute vec4 vertexGradient;
varying vec4 gradient;
attribute vec4 vertexCoord;
varying vec2 textureCoord;

vec4 addOffset(vec4 texCoord, vec2 offset, vec4 duvdxy)
{
    float dudx = duvdxy.x;
    float dvdx = duvdxy.y;
    float dudy = duvdxy.z;
    float dvdy = duvdxy.w;
    float u = (offset.x * dudx) + (offset.y * dudy);
    float v = (offset.x * dvdx) + (offset.y * dvdy);
    int specialCase = int(texCoord.z > 0.0) * (int(offset.x != 0.0) + int(offset.y != 0.0));
    return vec4(texCoord.x + u, texCoord.y + v, texCoord.z, float(specialCase));
}

void main()
{
    vec2 offset = (normalVector * 1.41421353816986083984375) / vec2(ubuf.matrixScale);
    vec4 param = vertexTexCoord;
    vec2 param_1 = offset;
    vec4 param_2 = vertexGradient;
    qt_TexCoord = addOffset(param, param_1, param_2);
    gradient = vertexGradient / vec4(ubuf.matrixScale);
    vec2 gradVertexCoord = (ubuf.gradientMatrix * vertexCoord).xy;
    textureCoord = vec2(gradVertexCoord.x / ubuf.boundsSize.x, gradVertexCoord.y / ubuf.boundsSize.y);
    gl_Position = ubuf.qt_Matrix * (vertexCoord + vec4(offset, 0.0, 0.0));
}

