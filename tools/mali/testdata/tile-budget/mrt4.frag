#version 300 es
// tools/shaders/es300/mrt3.frag with a fourth draw buffer: four RGBA8
// targets are 16 bytes a pixel in the tile buffer, the frame whose tile a
// G310v1-v3's halved budget halves (docs/notes/mali-g310-vs-g610.md).
precision mediump float;
layout(location = 0) out vec4 o0;
layout(location = 1) out vec4 o1;
layout(location = 2) out vec4 o2;
layout(location = 3) out vec4 o3;
void main()
{
    o0 = vec4(1.0, 0.0, 0.0, 1.0);
    o1 = vec4(0.0, 1.0, 0.0, 1.0);
    o2 = vec4(0.0, 0.0, 1.0, 1.0);
    o3 = vec4(1.0, 1.0, 1.0, 1.0);
}
