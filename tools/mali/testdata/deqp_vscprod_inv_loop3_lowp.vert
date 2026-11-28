attribute lowp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    lowp vec4 value = a_input;
    gl_Position = vec4(0.0, 0.0, 0.0, 0.0);
    v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);
    for (mediump int i = 0; i < 6; ++i)
    {
        value *= 1.1;
        gl_Position += vec4(value.xyz / 1.0 + a_input.xyz * 0.1, 1.0);
        v_unrelated = gl_Position.xyzx * a_input;
    }
}
