#version 100
/* A varying the vertex shader writes one lane of, on some paths only
 * (Qt's imageparticle_tabled: `tt.x = t;` in a branch): the lanes never
 * written are undefined, and the layout stores 0 there. */
attribute vec4 a_position;
attribute vec4 a_coords;
varying mediump vec2 tt;
varying mediump vec4 v_coords;
void main()
{
    gl_Position = a_position;
    v_coords = a_coords;
    if (a_coords.x > 0.0)
        tt.x = a_coords.y;
}
