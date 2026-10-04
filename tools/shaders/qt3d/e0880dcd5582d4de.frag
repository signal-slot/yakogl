#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 sampleDirections[16];
    float invTotalWeight;
    uint sampleCount;
};

uniform buf ubuf2;

uniform highp samplerCube environmentMap;

in vec3 localPos;
layout(location = 0) out vec4 FragColor;

void main()
{
    vec3 N = normalize(localPos);
    vec3 R = N;
    vec3 V = R;
    bvec3 _33 = bvec3(abs(N.z) < 0.999000012874603271484375);
    vec3 up = vec3(_33.x ? vec3(0.0, 0.0, 1.0).x : vec3(1.0, 0.0, 0.0).x, _33.y ? vec3(0.0, 0.0, 1.0).y : vec3(1.0, 0.0, 0.0).y, _33.z ? vec3(0.0, 0.0, 1.0).z : vec3(1.0, 0.0, 0.0).z);
    vec3 tangent = normalize(cross(up, N));
    vec3 bitangent = cross(N, tangent);
    mat3 tangentToWorld = mat3(vec3(tangent), vec3(bitangent), vec3(N));
    vec3 prefilteredColor = vec3(0.0);
    for (uint i = 0u; i < ubuf2.sampleCount; i++)
    {
        vec3 L = tangentToWorld * ubuf2.sampleDirections[i].xyz;
        prefilteredColor += (textureLod(environmentMap, L, ubuf2.sampleDirections[i].w).xyz * ubuf2.sampleDirections[i].z);
    }
    prefilteredColor *= ubuf2.invTotalWeight;
    FragColor = vec4(prefilteredColor, 1.0);
}

