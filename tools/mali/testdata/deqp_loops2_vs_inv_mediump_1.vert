attribute mediump vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    mediump vec4 value = a_input;
    for (mediump int i = 0; i < 13; ++i)
    {
        value *= 1.9;
        if (i == 11)
            v_unrelated = value;
    }
    gl_Position = vec4(value.xyz / 1.0e4 + a_input.xyz * 0.1, 1.0);
}
