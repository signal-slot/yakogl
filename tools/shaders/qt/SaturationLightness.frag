#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    float hue;
};

uniform buf _153;

varying vec2 qt_TexCoord0;

float hueToIntensity(float v1, float v2, inout float h)
{
    h = fract(h);
    if (h < 0.16666667163372039794921875)
    {
        return v1 + (((v2 - v1) * 6.0) * h);
    }
    else
    {
        if (h < 0.5)
        {
            return v2;
        }
        else
        {
            if (h < 0.666666686534881591796875)
            {
                return v1 + (((v2 - v1) * 6.0) * (0.666666686534881591796875 - h));
            }
        }
    }
    return v1;
}

vec3 HSLtoRGB(vec3 color)
{
    float h = color.x;
    float l = color.z;
    float s = color.y;
    if (s < 0.00390625)
    {
        return vec3(l);
    }
    float v2;
    if (l < 0.5)
    {
        v2 = l * (1.0 + s);
    }
    else
    {
        v2 = (l + s) - (s * l);
    }
    float v1 = (2.0 * l) - v2;
    float d = 0.3333333432674407958984375;
    float param = v1;
    float param_1 = v2;
    float param_2 = h + d;
    float _121 = hueToIntensity(param, param_1, param_2);
    float r = _121;
    float param_3 = v1;
    float param_4 = v2;
    float param_5 = h;
    float _129 = hueToIntensity(param_3, param_4, param_5);
    float g = _129;
    float param_6 = v1;
    float param_7 = v2;
    float param_8 = h - d;
    float _139 = hueToIntensity(param_6, param_7, param_8);
    float b = _139;
    return vec3(r, g, b);
}

void main()
{
    vec4 c = vec4(1.0);
    vec3 param = vec3(_153.hue, 1.0 - qt_TexCoord0.y, qt_TexCoord0.x);
    vec3 _170 = HSLtoRGB(param);
    c.x = _170.x;
    c.y = _170.y;
    c.z = _170.z;
    gl_FragData[0] = c * _153.qt_Opacity;
}

