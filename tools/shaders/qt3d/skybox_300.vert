#version 300 es

struct buf
{
    vec4 skyboxProperties;
    mat3 orientation;
    mat4 viewProjection;
    mat4 inverseProjection;
    mat3 viewMatrix;
};

uniform buf ubuf;

layout(location = 0) in vec3 attr_pos;
out vec3 eye_direction;

void main()
{
    gl_Position = vec4(attr_pos, 1.0);
    vec3 unprojected = (ubuf.inverseProjection * gl_Position).xyz;
    eye_direction = normalize(ubuf.viewMatrix * unprojected);
    eye_direction = normalize(ubuf.orientation * eye_direction);
    gl_Position.y *= ubuf.skyboxProperties.x;
}

