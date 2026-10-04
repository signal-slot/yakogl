#version 300 es

struct cbMain
{
    vec3 qt_cameraPosition;
    vec4 qt_material_base_color;
    float qt_material_clearcoat_fresnel_power;
    float qt_material_clearcoat_normal_strength;
    vec3 qt_material_emissive_color;
    vec4 qt_material_properties;
    vec4 qt_material_properties2;
    vec4 qt_material_properties3;
    vec4 qt_material_specular;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
};

uniform cbMain _250;

uniform highp sampler2D qt_boneTexture;

layout(location = 1) in vec3 attr_pos;
layout(location = 0) in ivec4 attr_joints;
layout(location = 2) in vec4 attr_weights;
out vec3 qt_varWorldPos;

mat4 qt_getTexMatrix(int index)
{
    int width = textureSize(qt_boneTexture, 0).x;
    int matId = index * 4;
    ivec2 p;
    p.x = matId % width;
    p.y = (matId - p.x) / width;
    mat4 ret;
    ret[0] = texelFetch(qt_boneTexture, p, 0);
    p.x = (matId + 1) % width;
    p.y = ((matId + 1) - p.x) / width;
    ret[1] = texelFetch(qt_boneTexture, p, 0);
    p.x = (matId + 2) % width;
    p.y = ((matId + 2) - p.x) / width;
    ret[2] = texelFetch(qt_boneTexture, p, 0);
    p.x = (matId + 3) % width;
    p.y = ((matId + 3) - p.x) / width;
    ret[3] = texelFetch(qt_boneTexture, p, 0);
    return ret;
}

mat4 qt_getSkinMatrix(ivec4 joints, vec4 weights)
{
    int param = joints.x * 2;
    mat4 _129 = qt_getTexMatrix(param) * weights.x;
    int param_1 = joints.y * 2;
    mat4 _137 = qt_getTexMatrix(param_1) * weights.y;
    mat4 _150 = mat4(_129[0] + _137[0], _129[1] + _137[1], _129[2] + _137[2], _129[3] + _137[3]);
    int param_2 = joints.z * 2;
    mat4 _159 = qt_getTexMatrix(param_2) * weights.z;
    mat4 _172 = mat4(_150[0] + _159[0], _150[1] + _159[1], _150[2] + _159[2], _150[3] + _159[3]);
    int param_3 = joints.w * 2;
    mat4 _181 = qt_getTexMatrix(param_3) * weights.w;
    return mat4(_172[0] + _181[0], _172[1] + _181[1], _172[2] + _181[2], _172[3] + _181[3]);
}

void main()
{
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    vec3 qt_vertNormal = vec3(0.0);
    vec3 qt_vertTangent = vec3(0.0);
    vec3 qt_vertBinormal = vec3(0.0);
    ivec4 qt_vertJoints = ivec4(0);
    vec4 qt_vertWeights = vec4(0.0);
    qt_vertJoints = attr_joints;
    qt_vertWeights = attr_weights;
    mat4 skinMat = mat4(vec4(1.0, 0.0, 0.0, 0.0), vec4(0.0, 1.0, 0.0, 0.0), vec4(0.0, 0.0, 1.0, 0.0), vec4(0.0, 0.0, 0.0, 1.0));
    if (any(notEqual(qt_vertWeights, vec4(0.0))))
    {
        ivec4 param = qt_vertJoints;
        vec4 param_1 = qt_vertWeights;
        skinMat = qt_getSkinMatrix(param, param_1);
        qt_vertPosition = skinMat * qt_vertPosition;
    }
    gl_Position = _250.qt_modelViewProjection * qt_vertPosition;
    vec3 qt_local_model_world_position = qt_vertPosition.xyz;
    qt_varWorldPos = qt_local_model_world_position;
}

