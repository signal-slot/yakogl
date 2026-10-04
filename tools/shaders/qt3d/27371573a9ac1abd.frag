#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    float minSide;
    uint multipliedAlpha;
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    vec4 qt_material_properties;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    mat3 qt_normalMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
    vec3 sliceMax;
    vec3 sliceMin;
    float stepAlpha;
    float stepLength;
    float tMax;
    float tMin;
};

uniform cbMain _94;

uniform highp sampler3D volume;
uniform highp sampler2D colormap;

layout(location = 0) out vec4 fragOutput;
in vec3 ray_direction_model;

bool ray_box_intersection(vec3 ray_origin, vec3 ray_direction, vec3 box_bottom, vec3 box_top, inout float t_0, inout float t_1)
{
    vec3 direction_inv = vec3(1.0) / ray_direction;
    vec3 t_top = direction_inv * (box_top - ray_origin);
    vec3 t_bottom = direction_inv * (box_bottom - ray_origin);
    vec3 t_min = min(t_top, t_bottom);
    vec2 t = max(t_min.xx, t_min.yz);
    t_0 = max(0.0, max(t.x, t.y));
    vec3 t_max = max(t_top, t_bottom);
    t = min(t_max.xx, t_max.yz);
    t_1 = min(t.x, t.y);
    bool _76 = t_1 >= 0.0;
    bool _82;
    if (_76)
    {
        _82 = t_1 >= t_0;
    }
    else
    {
        _82 = _76;
    }
    return _82;
}

void qt_customMain()
{
    fragOutput = vec4(0.0);
    vec3 ray_origin_model = (inverse(_94.qt_modelMatrix) * vec4(_94.qt_cameraPosition, 1.0)).xyz;
    vec3 top_sliced = (vec3(100.0) * _94.sliceMax) - vec3(50.0);
    vec3 bottom_sliced = (vec3(100.0) * _94.sliceMin) - vec3(50.0);
    vec3 param = ray_origin_model;
    vec3 param_1 = ray_direction_model;
    vec3 param_2 = bottom_sliced;
    vec3 param_3 = top_sliced;
    float param_4;
    float param_5;
    bool _141 = ray_box_intersection(param, param_1, param_2, param_3, param_4, param_5);
    float t_0 = param_4;
    float t_1 = param_5;
    if (!_141)
    {
        return;
    }
    vec3 ray_start = ((ray_origin_model + (ray_direction_model * t_0)) - vec3(-50.0)) / vec3(100.0);
    vec3 ray_stop = ((ray_origin_model + (ray_direction_model * t_1)) - vec3(-50.0)) / vec3(100.0);
    vec3 ray = ray_stop - ray_start;
    float ray_length = length(ray);
    vec3 step_vector = (ray * _94.stepLength) / vec3(ray_length);
    vec3 position = ray_start;
    float _237;
    while (ray_length > 0.0)
    {
        ray_length -= _94.stepLength;
        position += step_vector;
        float val = textureLod(volume, position, 0.0).x;
        bool _209 = val == 0.0;
        bool _218;
        if (!_209)
        {
            _218 = val < _94.tMin;
        }
        else
        {
            _218 = _209;
        }
        bool _227;
        if (!_218)
        {
            _227 = val > _94.tMax;
        }
        else
        {
            _227 = _218;
        }
        if (_227)
        {
            continue;
        }
        if (_94.multipliedAlpha != 0u)
        {
            _237 = val * _94.stepAlpha;
        }
        else
        {
            _237 = _94.stepAlpha;
        }
        float alpha = _237;
        vec4 val_color = vec4(textureLod(colormap, vec2(val, 0.5), 0.0).xyz, alpha);
        val_color.w = 1.0 - pow(max(0.0, 1.0 - val_color.w), 1.0);
        float _276 = fragOutput.w;
        vec4 _284 = fragOutput;
        vec3 _286 = _284.xyz + (val_color.xyz * ((1.0 - _276) * val_color.w));
        fragOutput.x = _286.x;
        fragOutput.y = _286.y;
        fragOutput.z = _286.z;
        fragOutput.w += ((1.0 - fragOutput.w) * val_color.w);
        if (fragOutput.w >= 0.949999988079071)
        {
            break;
        }
    }
}

void main()
{
    qt_customMain();
}

