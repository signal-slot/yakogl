/* Uniforms (L3 reference): a float then a mat4 (the matrix at words 1-16), a matrix product times a scalar (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform float u_a; uniform mat4 u_m; void main(){ gl_Position = u_m * a_pos * u_a; }
