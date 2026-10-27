/* Uniforms (L3 reference): one product twice (nir_opt_cse: one FMA) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u; void main(){ gl_Position = vec4(a_pos.x * u.x, a_pos.x * u.x, a_pos.zw); }
