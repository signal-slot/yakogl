#version 300 es
// gl_test case "cubelod", fragment stage: the direction across the
// whole +X face, interpolated per pixel.
precision highp float;
layout(location = 0) in vec4 a_pos;
out vec3 v_dir;
void main()
{
    gl_Position = vec4(a_pos.xy, 0.0, 1.0);
    v_dir = vec3(1.0, a_pos.y * 0.95, a_pos.x * 0.95);
}
