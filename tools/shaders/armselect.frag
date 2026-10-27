#version 450
/* One arm writes `v`, the other writes `w`: the shape that miscompiled.
 *
 * Both locals are live across the `if` and each is written by exactly one
 * arm, which is what `if (c) { x = ...; } else { y = ...; }` does in any real
 * shader. Selection reserved a register for the arm that does NOT write a
 * slot and then selected from it at the join, so on one side of the condition
 * the result was whatever that register happened to hold (gles2-core fce851e,
 * 2026-09-22; it was in 30 of the 137 sources that compile, the whole FemtoVG
 * set, and no gl_test case had a branch at all).
 *
 * Both sides are read back: with the flag set the colour is u_b, without it
 * u_a, and neither is the average the wrong compilation produced. */
layout(push_constant) uniform U { vec4 u_a; vec4 u_b; vec4 u_flag; };
layout(location = 0) out vec4 o_color;
void main() {
    vec4 v = u_a;
    vec4 w = u_b;
    if (u_flag.x > 0.5) { v = u_b; } else { w = u_a; }
    o_color = v * 0.5 + w * 0.5;
}
