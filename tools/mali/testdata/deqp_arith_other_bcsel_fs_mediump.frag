precision mediump float;
bool isOk (vec4 a, vec4 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying vec4 in0;
uniform vec4 ref_out0;
vec4 out0;


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
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}

float absDiff (vec4 a, vec4 b) { vec4 d = abs(a - b); return max(max(d.x, d.y), max(d.z, d.w)); }
vec4 get (in float arr[4]) { return vec4(arr[0], arr[1], arr[2], arr[3]); }
void set (out float arr[4], vec4 val) { arr[0] = val.x; arr[1] = val.y; arr[2] = val.z; arr[3] = val.w; }
void negate (inout float arr[4]) { set(arr, -get(arr)); }
bool test (in float arr[4], vec4 ref) { return (absDiff(get(arr), ref) < 0.1); }
bool isEqual (in float a[4], in float b[4]) { return (absDiff(get(a), get(b)) < 0.1); }
