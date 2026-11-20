/* dEQP-GLES2.functional.shaders.linkage.varying_differing_precision_3 (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape prec_mixed of
 * docs/notes/mali-deqp-varylink2.md: equal lanes beside distinct ones (var.z == var.w == -in0), a lowp vertex varying read mediump, refused at the link. */
attribute highp vec4 dEQP_Position;
attribute float in0;

varying lowp vec4 var;
void main()
{
    var = vec4(in0, 2.0*in0, -in0, -in0);
    gl_Position = dEQP_Position;

}
