precision highp int;
bool isOk (int a, int b)     { return (a == b); }
uniform int ref_out0;
int out0;


void main()
{
    const int a = 5-1 + 2*3 - 9/3;
    out0 = a;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
