/* dEQP-GLES2.functional.shaders.linkage.varying_differing_precision_3 (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape prec_mixed of
 * docs/notes/mali-deqp-varylink2.md: equal lanes beside distinct ones (var.z == var.w == -in0), a lowp vertex varying read mediump, refused at the link. */
precision mediump float;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
uniform float ref_out0;
float out0;

varying mediump vec4 var;
void main()
{
    out0 = var.x + var.y + var.z + var.w;
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
