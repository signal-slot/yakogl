precision mediump float;
attribute highp vec4 dEQP_Position;
attribute vec4 in0;
varying vec4 out0;


vec4 get (in float arr[4]);
void set (out float arr[4], vec4 val);
void negate (inout float arr[4]);
bool test (in float arr[4], vec4 ref);
bool isEqual (in float a[4], in float b[4]);

float func (in float a[4], in float b[4])
{
    a[0] = 2.123;
    a[2] = -4.123;
    return isEqual(a, b) ? 1.0 : -1.0;
}

float arr[4];

void main()
{
    set(arr, in0);
    out0 = in0 * func(arr, arr);
    gl_Position = dEQP_Position;

}

float absDiff (vec4 a, vec4 b) { vec4 d = abs(a - b); return max(max(d.x, d.y), max(d.z, d.w)); }
vec4 get (in float arr[4]) { return vec4(arr[0], arr[1], arr[2], arr[3]); }
void set (out float arr[4], vec4 val) { arr[0] = val.x; arr[1] = val.y; arr[2] = val.z; arr[3] = val.w; }
void negate (inout float arr[4]) { set(arr, -get(arr)); }
bool test (in float arr[4], vec4 ref) { return (absDiff(get(arr), ref) < 0.1); }
bool isEqual (in float a[4], in float b[4]) { return (absDiff(get(a), get(b)) < 0.1); }
