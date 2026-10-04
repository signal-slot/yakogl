#version 450
/* Position + colour + a separate per-vertex z: 9 input dwords against 8
 * output dwords, the shape Qt Quick's batchable shaders have (the vertex
 * input is larger than the output). Pairs with interp_color.frag. */
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_color;
layout(location = 2) in float a_z;
layout(location = 0) out vec4 v_color;
void main() {
    gl_Position = vec4(a_pos.xy, a_z, 1.0);
    v_color = a_color;
}
