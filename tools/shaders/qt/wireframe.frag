#version 100
#extension GL_OES_standard_derivatives : require
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
};

uniform buf ubuf;

varying vec3 barycentric;

void main()
{
    float f = min(barycentric.x, min(barycentric.y, barycentric.z));
    float d = fwidth(f * 1.5);
    float alpha = smoothstep(0.0, d, f);
    gl_FragData[0] = vec4(1.0, 0.20000000298023223876953125, 1.0, 1.0) * (1.0 - alpha);
}

