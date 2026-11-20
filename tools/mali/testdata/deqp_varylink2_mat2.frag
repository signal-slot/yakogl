#version 100
/* dEQP-GLES2.functional.shaders.conversions.scalar_to_matrix.float_to_mat2_vertex (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape mat2 of
 * docs/notes/mali-deqp-varylink2.md: a mat2 varying of one scalar (out0 = mat2(in0): the diagonal equal, the rest 0.0), refused at the link. */
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
