/* dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.int_to_mat2_vertex (vertex shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (data/gles2/shaders/conversions.test, Apache-2.0), laid out as the .qpa of
 * tools/mali/deqp-host-measure.sh records deqp_varylink2_mat2. A mat2 varying of an int built from a float attribute (GLES2 has no int attributes: int in0 = int(a_in0), as deqp_arith_*), the equal-lane shape of spike-equal-lanes. */
precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying mat2 out0;


void main()
{
    int in0 = int(a_in0);
    out0 = mat2(in0);
    gl_Position = dEQP_Position;

}
