precision mediump float;
precision mediump int;

bool isOk (mat4 a, mat4 b, float eps) { vec4 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), max(abs(a[2]-b[2]), abs(a[3]-b[3]))); return all(lessThanEqual(diff, vec4(eps))); }
varying float v_in0;
uniform mat4 ref_out0;
mat4 out0;


void main()
{
    int in0 = int(v_in0 * 1.0025);

    out0 = mat4(in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
