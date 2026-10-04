#version 300 es

struct buf
{
    mat4 mvp;
    mat4 prevMVP;
    mat4 instanceLocal;
    mat4 instanceGlobal;
    mat4 prevInstanceLocal;
    mat4 prevInstanceGlobal;
    vec4 currentAndLastJitter;
    float velocityAmount;
    int morphTargetCount;
    vec2 padding;
};

uniform buf ubuf;

uniform highp sampler2D qt_boneTexture;
uniform highp sampler2D lastBoneTexture;

layout(location = 0) in vec3 attr_pos;
layout(location = 2) in vec4 attr_weights;
layout(location = 1) in ivec4 attr_joints;
out vec4 vPosition;
out vec4 vPrevPosition;

mat4 getTexMatrix(int index, highp sampler2D textureStorage)
{
    int width = textureSize(textureStorage, 0).x;
    int matId = index * 4;
    ivec2 p;
    p.x = matId % width;
    p.y = (matId - p.x) / width;
    mat4 ret;
    ret[0] = texelFetch(textureStorage, p, 0);
    p.x = (matId + 1) % width;
    p.y = ((matId + 1) - p.x) / width;
    ret[1] = texelFetch(textureStorage, p, 0);
    p.x = (matId + 2) % width;
    p.y = ((matId + 2) - p.x) / width;
    ret[2] = texelFetch(textureStorage, p, 0);
    p.x = (matId + 3) % width;
    p.y = ((matId + 3) - p.x) / width;
    ret[3] = texelFetch(textureStorage, p, 0);
    return ret;
}

mat4 getSkinMatrix(ivec4 joints, vec4 weights, highp sampler2D boneTexture)
{
    int param = joints.x * 2;
    mat4 _130 = getTexMatrix(param, boneTexture) * weights.x;
    int param_1 = joints.y * 2;
    mat4 _138 = getTexMatrix(param_1, boneTexture) * weights.y;
    mat4 _151 = mat4(_130[0] + _138[0], _130[1] + _138[1], _130[2] + _138[2], _130[3] + _138[3]);
    int param_2 = joints.z * 2;
    mat4 _160 = getTexMatrix(param_2, boneTexture) * weights.z;
    mat4 _173 = mat4(_151[0] + _160[0], _151[1] + _160[1], _151[2] + _160[2], _151[3] + _160[3]);
    int param_3 = joints.w * 2;
    mat4 _182 = getTexMatrix(param_3, boneTexture) * weights.w;
    return mat4(_173[0] + _182[0], _173[1] + _182[1], _173[2] + _182[2], _173[3] + _182[3]);
}

void main()
{
    vec4 vp = vec4(attr_pos, 1.0);
    vec4 vpPre = vec4(attr_pos, 1.0);
    if (any(notEqual(attr_weights, vec4(0.0))))
    {
        ivec4 param = attr_joints;
        vec4 param_1 = attr_weights;
        mat4 skinMat = getSkinMatrix(param, param_1, qt_boneTexture);
        vp = skinMat * vp;
        ivec4 param_2 = attr_joints;
        vec4 param_3 = attr_weights;
        mat4 lastSkinMat = getSkinMatrix(param_2, param_3, lastBoneTexture);
        vpPre = lastSkinMat * vpPre;
    }
    vec4 _260 = ubuf.mvp * vp;
    vPosition = _260;
    gl_Position = _260;
    vPrevPosition = ubuf.prevMVP * vpPre;
}

