precision mediump float;
attribute highp vec4 dEQP_Position;
attribute vec4 a_in0;
varying vec4 v_out0;
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
    bvec4 in0 = bvec4(a_in0);

    arr[0] = !in0.x;
    arr[1] = !in0.y;
    arr[2] = !in0.z;
    arr[3] = !in0.w;
    func(arr);
    out0 = bvec4(arr[0], arr[1], arr[2], arr[3]);
    gl_Position = dEQP_Position;
v_out0 = vec4(out0);

}
