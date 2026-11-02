#version 100
/* es100/partial_varying.vert's varyings: tt.x where the vertex shader wrote
 * it, the rest undefined (the harness compares only defined lanes). */
precision mediump float;
varying mediump vec2 tt;
varying mediump vec4 v_coords;
void main()
{
    gl_FragColor = vec4(tt, v_coords.zw);
}
