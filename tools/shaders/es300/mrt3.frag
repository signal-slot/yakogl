#version 300 es
// gl_test 106 mrt3: three draw buffers, each its own colour, so a target
// that got another's output (or none) reads as the wrong colour.
precision mediump float;
layout(location = 0) out vec4 o0;
layout(location = 1) out vec4 o1;
layout(location = 2) out vec4 o2;
void main()
{
    o0 = vec4(1.0, 0.0, 0.0, 1.0);
    o1 = vec4(0.0, 1.0, 0.0, 1.0);
    o2 = vec4(0.0, 0.0, 1.0, 1.0);
}
