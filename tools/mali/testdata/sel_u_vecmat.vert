/* Uniforms (L3 reference): a vec4 attribute times a mat4 uniform (four dot products, w first) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform mat4 u_m; void main(){ gl_Position = a_pos * u_m; }
