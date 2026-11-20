precision mediump float;
precision mediump int;
bool isOk (int a, int b)     { return (a == b); }
varying float v_in0;
uniform int ref_out0;
int out0;


void func (inout int a)
{
    a = -a;
}

void main()
{
    int in0 = int(v_in0 * 1.0025);

    int f = 1;
    func(f);
    out0 = f * in0;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
