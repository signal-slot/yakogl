#version 100
#extension GL_OES_standard_derivatives : require
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float matrixScale;
    float opacity;
    float debug;
    float reserved3;
    vec4 color;
};

uniform buf ubuf;

varying vec4 qt_TexCoord;
varying vec4 gradient;

vec4 baseColor()
{
    return vec4(ubuf.color.xyz, 1.0) * ubuf.color.w;
}

void main()
{
    float f = (qt_TexCoord.z * ((qt_TexCoord.x * qt_TexCoord.x) - qt_TexCoord.y)) + ((1.0 - abs(qt_TexCoord.z)) * (qt_TexCoord.x - qt_TexCoord.y));
    float _ddx = dFdx(f);
    float _ddy = dFdy(f);
    float df = length(vec2(_ddx, _ddy));
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

