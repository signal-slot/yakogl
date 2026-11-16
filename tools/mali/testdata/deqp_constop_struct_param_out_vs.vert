attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;

struct S {
    highp vec3 red;
    highp vec3 blue;
};

void modify (out S s)
{
    s.red = vec3(1.0, 0.0, 0.0);
    s.blue = vec3(0.0, 0.0, 1.0);
}

void main (void)
{
    S s;
    modify(s);
    v_color = vec4(0.0, 0.0, 0.0, 1.0);
    if (s.red == vec3(1.0, 0.0, 0.0) && s.blue == vec3(0.0, 0.0, 1.0))
        v_color = vec4(1.0, 1.0, 1.0, 1.0);
    gl_Position = a_position;
}
