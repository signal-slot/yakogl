#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float matrixScale;
    float opacity;
    float debug;
    float reserved3;
    mat4 gradientMatrix;
    vec2 translationPoint;
    float angle;
};

uniform buf ubuf;

uniform highp sampler2D gradTabTexture;

varying vec2 coord;
varying vec4 qt_TexCoord;
varying vec4 gradient;

vec4 baseColor()
{
    float t;
    if (abs(coord.y) == abs(coord.x))
    {
        t = (atan((-coord.y) + 0.00200000009499490261077880859375, coord.x) + ubuf.angle) * 0.15915493667125701904296875;
    }
    else
    {
        t = (atan(-coord.y, coord.x) + ubuf.angle) * 0.15915493667125701904296875;
    }
    return texture2D(gradTabTexture, vec2(t - floor(t), 0.5));
}

void main()
{
    float f = (qt_TexCoord.z * ((qt_TexCoord.x * qt_TexCoord.x) - qt_TexCoord.y)) + ((1.0 - abs(qt_TexCoord.z)) * (qt_TexCoord.x - qt_TexCoord.y));
    float dudx = gradient.x;
    float dvdx = gradient.y;
    float dudy = gradient.z;
    float dvdy = gradient.w;
    float dfx_curve = ((2.0 * qt_TexCoord.x) * dudx) - dvdx;
    float dfy_curve = ((2.0 * qt_TexCoord.x) * dudy) - dvdy;
    float dfx_line = dudx - dvdx;
    float dfy_line = dudy - dvdy;
    float dfx = (qt_TexCoord.z * dfx_curve) + ((1.0 - abs(qt_TexCoord.z)) * dfx_line);
    float dfy = (qt_TexCoord.z * dfy_curve) + ((1.0 - abs(qt_TexCoord.z)) * dfy_line);
    float df = length(vec2(dfx, dfy));
    float isLine = 1.0 - abs(qt_TexCoord.z);
    float isCurve = 1.0 - isLine;
    float debugR = isCurve * min(1.0, 1.0 - qt_TexCoord.z);
    float debugG = isLine;
    float debugB = (isCurve * min(1.0, 1.0 - (qt_TexCoord.z * (-1.0)))) + debugG;
    vec3 debugColor = vec3(debugR, debugG, debugB);
    int specialCaseMask = 1 - (int(qt_TexCoord.w != 0.0) * (int(qt_TexCoord.x < 0.0) + int(qt_TexCoord.x > 1.0)));
    float fillCoverage = clamp(0.5 + (f / df), 0.0, 1.0) * float(specialCaseMask);
    gl_FragData[0] = mix(baseColor() * fillCoverage, vec4(debugColor, 1.0), vec4(ubuf.debug)) * ubuf.opacity;
}

