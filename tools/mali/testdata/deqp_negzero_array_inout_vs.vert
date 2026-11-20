/* dEQP-GLES2.functional.shaders.functions.array_arguments.copy_local_inout_on_call_vertex,
 * rebuilt from VK-GL-CTS 93bca018 (Apache-2.0): the "both" source of the case
 * copy_local_inout_on_call (data/gles3/shaders/functions.test:2547-2590 at the
 * same commit, its #version 300 es line dropped: the gles2 data file was not
 * reachable offline) through glsShaderLibraryCase.cpp's specializeVertexShader. */
precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec4 in0;
varying vec4 out0;


vec4 get (in float arr[4]);
void set (out float arr[4], vec4 val);
void negate (inout float arr[4]);
bool test (in float arr[4], vec4 ref);
bool isEqual (in float a[4], in float b[4]);

float func (inout float a[4], inout float b[4])
{
    negate(a);
    return isEqual(a, b) ? 1.0 : -1.0;
}

void main()
{
    float arr[4];
    set(arr, in0);
    float m = func(arr, arr); // returns -1.0
    float n = float(test(arr, in0) || test(arr, -in0));
    out0 = in0 * m * n;
    gl_Position = dEQP_Position;

}

float absDiff (vec4 a, vec4 b) { vec4 d = abs(a - b); return max(max(d.x, d.y), max(d.z, d.w)); }
vec4 get (in float arr[4]) { return vec4(arr[0], arr[1], arr[2], arr[3]); }
void set (out float arr[4], vec4 val) { arr[0] = val.x; arr[1] = val.y; arr[2] = val.z; arr[3] = val.w; }
void negate (inout float arr[4]) { set(arr, -get(arr)); }
bool test (in float arr[4], vec4 ref) { return (absDiff(get(arr), ref) < 0.1); }
bool isEqual (in float a[4], in float b[4]) { return (absDiff(get(a), get(b)) < 0.1); }
