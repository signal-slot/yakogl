precision highp int;
attribute highp vec4 dEQP_Position;
varying float v_out0;
bool out0;


void main()
{
    const bool a = (!false || false) && (true ^^ false);
    out0 = a;
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
