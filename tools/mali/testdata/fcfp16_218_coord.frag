#version 300 es
/* gl_test 218 shadowcmp's position arithmetic without its two shadow
 * look-ups: the coordinates each look-up takes, stored as the colour. */
precision mediump float;
out vec4 o_color;
void main() {
    float right = step(32.0, gl_FragCoord.x);
    vec2 b = vec2(floor((gl_FragCoord.x - 32.0 * right) / 8.0), floor(gl_FragCoord.y / 16.0));
    o_color = vec4((b + 0.5) / 4.0, (b + 1.0) / 4.0);
}
