precision highp float;
attribute highp vec4 dEQP_Position;
varying float v_out0;
int out0;

void main()
{
    out0 = 123;
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
