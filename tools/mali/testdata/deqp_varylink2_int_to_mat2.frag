#version 100
/* dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.int_to_mat2_vertex (fragment shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (data/gles2/shaders/conversions.test, Apache-2.0), laid out as the .qpa of
 * tools/mali/deqp-host-measure.sh records deqp_varylink2_mat2. A mat2 varying of an int built from a float attribute (GLES2 has no int attributes: int in0 = int(a_in0), as deqp_arith_*), the equal-lane shape of spike-equal-lanes. */
precision mediump float;
precision mediump int;

bool isOk (mat2 a, mat2 b, float eps) { vec2 diff = max(abs(a[0]-b[0]), abs(a[1]-b[1])); return all(lessThanEqual(diff, vec2(eps))); }

varying mat2 out0;
uniform mat2 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
