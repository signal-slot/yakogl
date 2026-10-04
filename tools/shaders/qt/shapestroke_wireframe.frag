#version 100
#extension GL_OES_standard_derivatives : require
precision highp float;
precision highp int;

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

varying vec3 barycentric;

void main()
{
    float f = min(barycentric.x, min(barycentric.y, barycentric.z));
    float d = fwidth(f * 1.5);
    float alpha = smoothstep(0.0, d, f);
    gl_FragData[0] = vec4(1.0, 0.4000000059604644775390625, 0.100000001490116119384765625, 1.0) * (1.0 - alpha);
}

