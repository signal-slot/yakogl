/* Uniforms (L3 reference): a vec3 then a float (the float at word 3) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec3 u_c; uniform float u_d; void main(){ gl_Position = vec4(a_pos.xyz + u_c, a_pos.w * u_d); }
