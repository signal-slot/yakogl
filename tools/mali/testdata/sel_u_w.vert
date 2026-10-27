/* Uniforms (L3 reference): the position's w a uniform (FRCP of a FAU word) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform float u_s; void main(){ gl_Position = vec4(a_pos.xyz * u_s, u_s); }
