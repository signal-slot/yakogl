precision mediump float;
attribute highp vec4 dEQP_Position;
attribute vec4 in0;
varying vec4 out0;


float func (in float a[4])
{
    a[0] = -1.0;
    a[2] = -4.0;
    a[3] = -3.0 * a[1];
    return a[0];
}

float arr[4];

void main()
{
    arr[0] = in0.x;
    arr[1] = in0.y;
    arr[2] = in0.z;
    arr[3] = in0.w;
    float f = func(arr);
    out0 = f * vec4(arr[0], arr[1], arr[2], arr[3]);
    gl_Position = dEQP_Position;

}
