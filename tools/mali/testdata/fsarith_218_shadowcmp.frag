#version 300 es
precision mediump float;
uniform mediump sampler2DShadow u_near;
uniform mediump sampler2DShadow u_lin;
out vec4 o_color;
void main() {
    float right = step(32.0, gl_FragCoord.x);
    vec2 b = vec2(floor((gl_FragCoord.x - 32.0 * right) / 8.0), floor(gl_FragCoord.y / 16.0));
    float n = texture(u_near, vec3((b + 0.5) / 4.0, 0.5));
    float l = texture(u_lin, vec3((b + 1.0) / 4.0, 0.5));
    float r = right > 0.5 ? l : n;
    o_color = vec4(r, r, r, 1.0);
}
