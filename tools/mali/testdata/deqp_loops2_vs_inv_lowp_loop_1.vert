attribute lowp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    lowp vec4 value = a_input;
    for (mediump int i = 0; i < 6; ++i)
    {
        value *= 1.1;
        if (i == 2)
            v_unrelated = value;
    }
    gl_Position = vec4(value.xyz / 2.0 + a_input.xyz * 0.1, 1.0);
}
