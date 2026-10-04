#version 100

struct buf
{
    mat4 qt_Matrix;
    float matrixScale;
    float opacity;
    float devicePixelRatio;
    float strokeWidth;
    vec4 strokeColor;
    float debug;
    float reserved5;
    float reserved6;
    float reserved7;
};

uniform buf ubuf;

attribute vec3 normalExt;
attribute vec4 vertexCoord;
varying vec3 barycentric;
attribute vec3 vertexBarycentric;
attribute float _qt_order;

void main()
{
    vec4 normalV = vec4(normalExt.xy, 0.0, 0.0);
    vec4 P = (vertexCoord + (normalV * abs(ubuf.strokeWidth))) + ((normalV * 1.41421353816986083984375) / vec4(ubuf.matrixScale));
    if (ubuf.strokeWidth < 0.0)
    {
        float adjStrokeWidth = (abs(ubuf.strokeWidth) * ubuf.devicePixelRatio) / ubuf.matrixScale;
        float strokeDiff = (max(ubuf.devicePixelRatio / ubuf.matrixScale, adjStrokeWidth) * ((ubuf.matrixScale / ubuf.devicePixelRatio) - 1.0)) / 2.0;
        P -= (normalV * strokeDiff);
    }
    barycentric = vertexBarycentric;
    gl_Position = ubuf.qt_Matrix * P;
    gl_Position.z = _qt_order * gl_Position.w;
}

