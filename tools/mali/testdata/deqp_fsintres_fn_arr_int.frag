precision mediump float;
precision mediump int;
bool isOk (ivec4 a, ivec4 b) { return (a == b); }
varying vec4 v_in0;
uniform ivec4 ref_out0;
ivec4 out0;


int func (in int a[4])
{
    a[0] = -1;
    a[2] = -4;
    a[3] = -3 * a[1];
    return a[0];
}

int arr[4];

void main()
{
    ivec4 in0 = ivec4(v_in0 * 1.0025);

    arr[0] = in0.x;
    arr[1] = in0.y;
    arr[2] = in0.z;
    arr[3] = in0.w;
    int f = func(arr);
    out0 = f * ivec4(arr[0], arr[1], arr[2], arr[3]);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}

