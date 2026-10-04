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
varying vec4 P;
attribute vec4 vertexCoord;
varying vec2 A;
attribute vec2 inA;
varying vec2 B;
attribute vec2 inB;
varying vec2 C;
attribute vec2 inC;
varying vec4 HGOW;

float qdot(vec2 a, vec2 b)
{
    return (a.x * b.x) + (a.y * b.y);
}

void main()
{
    vec4 normalVector = vec4(normalExt.xy, 0.0, 0.0);
    P = vertexCoord + ((normalVector * 1.41421353816986083984375) / vec4(ubuf.matrixScale));
    float adjStrokeWidth = abs(ubuf.strokeWidth);
    P += (normalVector * adjStrokeWidth);
    if (ubuf.strokeWidth < 0.0)
    {
        adjStrokeWidth *= (ubuf.devicePixelRatio / ubuf.matrixScale);
        float strokeDiff = (max(ubuf.devicePixelRatio / ubuf.matrixScale, adjStrokeWidth) * ((ubuf.matrixScale / ubuf.devicePixelRatio) - 1.0)) / 2.0;
        P -= (normalVector * strokeDiff);
    }
    adjStrokeWidth *= normalExt.z;
    A = inA;
    B = inB;
    C = inC;
    vec2 param = A;
    vec2 param_1 = A;
    float a = (-2.0) * qdot(param, param_1);
    vec2 param_2 = A;
    vec2 param_3 = B;
    float b = (-3.0) * qdot(param_2, param_3);
    vec2 param_4 = A;
    vec2 param_5 = P.xy;
    vec2 param_6 = B;
    vec2 param_7 = B;
    vec2 param_8 = A;
    vec2 param_9 = C;
    float c = ((2.0 * qdot(param_4, param_5)) - qdot(param_6, param_7)) - (2.0 * qdot(param_8, param_9));
    vec2 param_10 = B;
    vec2 param_11 = P.xy;
    vec2 param_12 = B;
    vec2 param_13 = C;
    float d = qdot(param_10, param_11) - qdot(param_12, param_13);
    float H = (((3.0 * a) * c) - (b * b)) / ((3.0 * a) * a);
    float G = (((((2.0 * b) * b) * b) - (((9.0 * a) * b) * c)) + (((27.0 * a) * a) * d)) / (((27.0 * a) * a) * a);
    HGOW = vec4(H, G, b / (3.0 * a), adjStrokeWidth);
    gl_Position = ubuf.qt_Matrix * P;
}

