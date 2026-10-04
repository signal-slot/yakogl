#version 100
precision highp float;
precision highp int;

uniform highp sampler2D tex;

varying vec2 uv_coord;

vec3 qt_exposure(vec3 color, float exposure)
{
    return color;
}

vec3 qt_tonemap(vec3 color)
{
    return color;
}

void main()
{
    vec4 color = texture2D(tex, uv_coord);
    vec3 param = color.xyz;
    float param_1 = 1.0;
    vec3 param_2 = qt_exposure(param, param_1);
    gl_FragData[0] = vec4(qt_tonemap(param_2), 1.0);
}

