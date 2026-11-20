precision mediump float;
precision mediump int;

bool isOk (mat3 a, mat3 b, float eps) { vec3 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), abs(a[2]-b[2])); return all(lessThanEqual(diff, vec3(eps))); }
varying float v_in0;
varying float in1;
varying float v_in2;
varying vec2 in3;
varying float v_in4;
varying vec2 v_in5;
varying float in6;
uniform mat3 ref_out0;
mat3 out0;


void main()
{
    bool in0 = bool(v_in0);
int in2 = int(v_in2 * 1.0025);
bool in4 = bool(v_in4);
bvec2 in5 = bvec2(v_in5);

    out0 = mat3(in0, in1, in2, in3, in4, in5, in6);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
