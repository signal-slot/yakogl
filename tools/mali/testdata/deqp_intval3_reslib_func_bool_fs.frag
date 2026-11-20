precision mediump float;
bool isOk (bool a, bool b)   { return (a == b); }
varying float v_in0;
uniform bool ref_out0;
bool out0;


bool func (bool a)
{
    return !a;
}

void main()
{
    bool in0 = bool(v_in0);

    out0 = func(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
