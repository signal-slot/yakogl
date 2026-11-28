precision mediump float;
bool isOk (bvec4 a, bvec4 b) { return (a == b); }
varying vec4 v_in0;
uniform bvec4 ref_out0;
bvec4 out0;


bool func (in bool a[4])
{
    a[0] = false;
    a[2] = true;
    a[3] = !a[1];
    return a[0];
}

bool arr[4];

void main()
{
    bvec4 in0 = bvec4(v_in0);

    arr[0] = !in0.x;
    arr[1] = !in0.y;
    arr[2] = !in0.z;
    arr[3] = !in0.w;
    func(arr);
    out0 = bvec4(arr[0], arr[1], arr[2], arr[3]);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
