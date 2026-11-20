#version 100
/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec2_grg_vertex (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape swz_mixed of
 * docs/notes/mali-deqp-varylink2.md: equal lanes beside a distinct lane (out0 = in0.grg: lanes 0 and 2 equal), refused at the link. */
precision mediump float;
precision mediump int;

bool isOk (vec3 a, vec3 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }

varying vec3 out0;
uniform vec3 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
