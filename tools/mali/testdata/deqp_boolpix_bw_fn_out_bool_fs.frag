precision mediump float;
bool isOk (bool a, bool b)   { return (a == b); }
varying float v_in0;
uniform bool ref_out0;
bool out0;


void func (out bool a)
{
    a = false;
}

void main()
{
    bool in0 = bool(v_in0);

    bool f = true;
    func(f);
    out0 = (in0 == f);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
