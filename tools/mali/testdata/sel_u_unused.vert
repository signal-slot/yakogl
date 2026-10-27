/* Uniforms (L3 reference): an unused uniform declared first takes no word (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u_unused; uniform vec4 u_used; void main(){ gl_Position = a_pos + u_used; }
