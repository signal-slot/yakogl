attribute mediump vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    mediump vec4 value = a_input;
    v_unrelated = vec4(0.0, 0.0, -1.0, 1.0);
    for (mediump int i = 0; i < 13; ++i)
    {
        value *= 1.9;
        if (i == 11)
            gl_Position = a_input + 0.05 * vec4(fract(value.xyz / 1.0e2), 1.0);
        else
            v_unrelated = value + a_input;
    }
}
