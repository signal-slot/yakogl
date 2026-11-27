#version 100
/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec4_yyw_vertex (fragment shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (data/gles2/shaders/swizzles.test, Apache-2.0), laid out as the .qpa of
 * tools/mali/deqp-host-measure.sh records the swz_mixed pair. Shape swz_mixed
 * of docs/notes/mali-deqp-varylink2.md, a non-aligned swizzle (out0 = in0.yyw). */
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
