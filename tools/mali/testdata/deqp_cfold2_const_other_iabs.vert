precision highp int;
attribute highp vec4 dEQP_Position;
varying float v_out0;
int out0;


void main()
{
    const int a = 5-1 + 2*3 - 9/3;
    out0 = a;
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
