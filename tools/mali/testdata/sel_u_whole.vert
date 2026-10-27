/* Uniforms (L3 reference): a vec4 uniform as the position, no attribute (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
uniform vec4 u_p; void main(){ gl_Position = u_p; }
