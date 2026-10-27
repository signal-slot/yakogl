#version 100
/* gl_PointSize written in both arms of an if, merged as gl_Position is
 * (prepare.rs if-conversion), and kept in the VPM for every primitive
 * (select.rs select_vertex, the point size row). es100/va.frag draws
 * v_color. */
attribute vec4 a_position;
attribute vec4 a_color;
varying mediump vec4 v_color;
void main()
{
    gl_Position = a_position;
    v_color = a_color;
    if (a_position.x > 0.0)
        gl_PointSize = 2.0 + a_color.x;
    else
        gl_PointSize = 5.0 * a_color.y;
}
