#version 100
/* dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.float_to_mat4_vertex (fragment shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (data/gles2/shaders/conversions.test, Apache-2.0), laid out as the .qpa of
 * tools/mali/deqp-host-measure.sh records deqp_varylink2_mat2. A mat4 varying of one scalar (out0 = mat4(in0)), the equal-lane shape of spike-equal-lanes. */
precision mediump float;
precision mediump int;

bool isOk (mat4 a, mat4 b, float eps) { vec4 diff = max(max(abs(a[0]-b[0]), abs(a[1]-b[1])), max(abs(a[2]-b[2]), abs(a[3]-b[3]))); return all(lessThanEqual(diff, vec4(eps))); }

varying mat4 out0;
uniform mat4 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
