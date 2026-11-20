attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform lowp float uf_third;
uniform lowp float uf_one;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    for (lowp float ndx = 0.05; ndx < uf_one; ndx += uf_third)
    {
        res = res.yzwx;
    }
    v_color = res.rgb;
}
