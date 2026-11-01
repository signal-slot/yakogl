#version 300 es
// gl_test case 89 "flatlast": flat varyings take the PROVOKING vertex's
// value, and GL's is the LAST vertex of the primitive (ES 3.0 2.18). A float
// vector and a uint (Qt Quick 3D's "flat out uint instanceIndex" shape), both
// from the vertex's attribute, so nothing but the provoking vertex decides.
layout(location = 0) in vec4 pos;
layout(location = 1) in vec4 col;
flat out vec4 v_flat;
flat out uint v_idx;
void main()
{
    v_flat = col;
    v_idx = uint(col.a * 4.0 + 0.5);
    gl_Position = pos;
}
