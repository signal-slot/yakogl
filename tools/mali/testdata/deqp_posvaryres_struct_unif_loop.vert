attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_two;

struct S {
    mediump float    a;
    mediump int        b;
};
uniform S s[3];

void main (void)
{
    mediump float rgb[3];
    int alpha = 0;
    for (int i = 0; i < 3; i++)
    {
        rgb[i] = s[2-i].a;
        alpha += s[i].b;
    }
    v_color = vec4(rgb[0], rgb[1], rgb[2], alpha);
    gl_Position = a_position;
}
