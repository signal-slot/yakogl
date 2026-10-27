#version 300 es
// gl_test case 89 "flatlast": the flat colour, and the flat uint as alpha.
precision mediump float;
flat in vec4 v_flat;
flat in uint v_idx;
layout(location = 0) out vec4 fragColor;
void main()
{
    fragColor = vec4(v_flat.rgb, float(v_idx) / 4.0);
}
