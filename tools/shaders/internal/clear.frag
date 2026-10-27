#version 450
/* The clear colour, from the one uniform the backend writes per clear. */
layout(push_constant) uniform Clear {
    vec4 u_color;
};
layout(location = 0) out vec4 o_color;
void main() {
    o_color = u_color;
}
