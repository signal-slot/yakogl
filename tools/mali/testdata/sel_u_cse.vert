/* Uniforms (L3 reference): one uniform in two expressions (one load: nir_opt_cse) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u_v; void main(){ gl_Position = a_pos * u_v + u_v; }
