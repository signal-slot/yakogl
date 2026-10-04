#version 300 es
precision mediump float;
uniform samplerCube cube;
uniform vec3 axis;
in vec2 v_xy;
layout(location = 0) out vec4 fragColor;
void main()
{
    // the major axis picks the face; xy spread the sample across it
    vec3 d = axis + vec3(v_xy.x, v_xy.y, 0.0) * 0.8;
    fragColor = textureLod(cube, d, 0.0);
}
