/* dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.float_to_mat2_vertex (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape mat2 of
 * docs/notes/mali-deqp-varylink2.md: a mat2 varying of one scalar (out0 = mat2(in0): the diagonal equal, the rest 0.0), refused at the link. */
precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float in0;
varying mat2 out0;


void main()
{
    
    out0 = mat2(in0);
    gl_Position = dEQP_Position;

}
