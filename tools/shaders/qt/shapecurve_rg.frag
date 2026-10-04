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
    vec2 focalToCenter;
    float centerRadius;
    float focalRadius;
};

uniform buf ubuf;

uniform highp sampler2D gradTabTexture;

varying vec2 coord;
varying vec4 qt_TexCoord;
varying vec4 gradient;

vec4 baseColor()
{
    float rd = ubuf.centerRadius - ubuf.focalRadius;
    float b = 2.0 * ((rd * ubuf.focalRadius) + dot(coord, ubuf.focalToCenter));
    float fmp2_m_radius2 = (((-ubuf.focalToCenter.x) * ubuf.focalToCenter.x) - (ubuf.focalToCenter.y * ubuf.focalToCenter.y)) + (rd * rd);
    float inverse_2_fmp2_m_radius2 = 1.0 / (2.0 * fmp2_m_radius2);
    float det = (b * b) - ((4.0 * fmp2_m_radius2) * ((ubuf.focalRadius * ubuf.focalRadius) - dot(coord, coord)));
    vec4 result = vec4(0.0);
    if (det >= 0.0)
    {
        float detSqrt = sqrt(det);
        float w = max(((-b) - detSqrt) * inverse_2_fmp2_m_radius2, ((-b) + detSqrt) * inverse_2_fmp2_m_radius2);
        if ((ubuf.focalRadius + (w * (ubuf.centerRadius - ubuf.focalRadius))) >= 0.0)
        {
            result = texture2D(gradTabTexture, vec2(w, 0.5));
        }
    }
    return result;
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

