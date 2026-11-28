/* dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.float_to_mat4_vertex (vertex shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (data/gles2/shaders/conversions.test, Apache-2.0), laid out as the .qpa of
 * tools/mali/deqp-host-measure.sh records deqp_varylink2_mat2. A mat4 varying of one scalar (out0 = mat4(in0)), the equal-lane shape of spike-equal-lanes. */
precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float in0;
varying mat4 out0;


void main()
{
    
    out0 = mat4(in0);
    gl_Position = dEQP_Position;

}
