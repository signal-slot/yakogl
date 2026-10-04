#version 450
/* Position passthrough for the framebuffer-load quad (pairs with fbload.frag). */
layout(location = 0) in vec4 a_pos;
void main() {
    gl_Position = a_pos;
}
