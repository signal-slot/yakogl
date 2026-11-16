attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform highp float uf_one;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    for (highp float ndx = 0.05; ndx < 1.0; ndx += 0.333333)
    {
        res = res.yzwx;
    }
    v_color = res.rgb;
}
