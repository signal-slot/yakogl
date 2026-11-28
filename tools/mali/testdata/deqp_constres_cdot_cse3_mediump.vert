attribute mediump vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    mediump float x = a_input.x * 0.2;
    mediump vec4 a = a_input.xxyx * 0.7;
    mediump vec4 b = a_input.yxyz * 0.7;
    mediump vec4 c = a_input.zxyx * 0.5;
    v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);
    mediump vec4 g = x * (a + b + c);
    gl_Position = a_input + g;
}
