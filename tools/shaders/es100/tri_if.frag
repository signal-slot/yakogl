#version 100
/* v3d_triangle's if variant (TRI_IF): an if on an interpolated value whose
 * threshold crosses the triangle, each arm writing a lane the other does
 * not, so one picture shows both sides of the condition and, on each side,
 * a lane that keeps its value from before the if. */
precision mediump float;
varying vec4 v_color;
void main() {
    vec4 c = v_color;
    if (v_color.r > 0.5) {
        c.g = 1.0;
    } else {
        c.b = 0.25;
    }
    gl_FragColor = c;
}
