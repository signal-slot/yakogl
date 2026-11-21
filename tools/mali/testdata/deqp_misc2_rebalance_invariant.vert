attribute highp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    highp vec4 a = 1.0e20 * a_input.zzxx + a_input.xzxy - 1.0e20 * a_input.zzxx;
    highp vec4 b = 1.0e20 * a_input.zzxx;
    highp vec4 c = b - 1.0e20 * a_input.zzxx + a_input.xzxy;
    highp vec4 d = (1.0e9 * a_input.yzxx) * (1.0e9 * a_input.yzzw) * (1.1*1.0e-9 * a_input.yzxx) * (1.0e-9 * a_input.xzzy);
    highp vec4 e = ((1.0e9 * a_input.yzxx) * (1.1*1.0e-9 * a_input.yzxx)) * ((1.0e-9 * a_input.xzzy) * (1.0e9 * a_input.yzzw));
    v_unrelated = a + b + c + d + e;
    gl_Position = a_input + fract(c) + e;
}
