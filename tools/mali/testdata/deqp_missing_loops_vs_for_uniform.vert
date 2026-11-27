attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_three;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    for (mediump int ndx = 0; ndx < ui_three; ndx++)
    {
        res = res.yzwx;
    }
    v_color = res.rgb;
}
