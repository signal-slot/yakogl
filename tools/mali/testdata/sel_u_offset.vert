/* Uniforms (L3 reference): a whole vec4 attribute plus a vec4 uniform (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u_offset; void main(){ gl_Position = a_pos + u_offset; }
