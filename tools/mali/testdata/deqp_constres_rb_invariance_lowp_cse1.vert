attribute lowp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    lowp vec4 a = 0.9 * a_input.zzxx + a_input.xzxy - 0.9 * a_input.zzxx;
    lowp vec4 b = 0.9 * a_input.zzxx;
    lowp vec4 c = b - 0.9 * a_input.zzxx + a_input.xzxy;
    lowp vec4 d = (1.15 * a_input.yzxx) * (1.15 * a_input.yzzw) * (1.1*0.87 * a_input.yzxx) * (0.87 * a_input.xzzy);
    lowp vec4 e = ((1.15 * a_input.yzxx) * (1.1*0.87 * a_input.yzxx)) * ((0.87 * a_input.xzzy) * (1.15 * a_input.yzzw));
    v_unrelated = a + b + c + d + e;
    gl_Position = a_input + fract(c) + e;
}
