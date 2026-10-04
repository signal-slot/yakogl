#version 100
precision highp float;
precision highp int;

uniform highp sampler2D tex;

varying vec2 uv_coord;

vec3 qt_exposure(vec3 color, float exposure)
{
    return vec3(1.0) - exp((-color) * exposure);
}

vec3 qt_tonemapHejlDawson(inout vec3 color)
{
    color = max(vec3(0.0), color - vec3(0.0040000001899898052215576171875));
    return (color * ((color * 6.19999980926513671875) + vec3(0.5))) / ((color * ((color * 6.19999980926513671875) + vec3(1.7000000476837158203125))) + vec3(0.0599999986588954925537109375));
}

vec3 qt_tonemap(vec3 color)
{
    vec3 param = color;
    vec3 _52 = qt_tonemapHejlDawson(param);
    return _52;
}

void main()
{
    vec4 color = texture2D(tex, uv_coord);
    vec3 param = color.xyz;
    float param_1 = 1.0;
    vec3 param_2 = qt_exposure(param, param_1);
    gl_FragData[0] = vec4(qt_tonemap(param_2), 1.0);
}

