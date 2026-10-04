#version 300 es
// gl_test case "cubelod", fragment stage: textureLod on a cube at an
// explicit level, per pixel; every pixel must read the level's flat +X
// colour.
precision highp float;
uniform highp samplerCube u_s;
uniform float u_lod;
in vec3 v_dir;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = textureLod(u_s, v_dir, u_lod);
}
