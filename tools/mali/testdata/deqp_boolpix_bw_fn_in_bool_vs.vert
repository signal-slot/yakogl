precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying float v_out0;
bool out0;


bool func (in bool a)
{
    a = !a;
    return a;
}

void main()
{
    bool in0 = bool(a_in0);

    bool f = in0;
    bool g = func(f);
    out0 = (f != g);
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
