#version 100
/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec3_xyyx_vertex (fragment shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (Apache-2.0), laid out as the .qpa of tools/mali/deqp-host-measure.sh records
 * the neighbouring deqp_varylink2_* pairs. Shape equal-lanes of spike-equal-lanes: out0 = in0.xyyx. */
precision mediump float;

bool isOk (vec4 a, vec4 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }

varying vec4 out0;
uniform vec4 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
