/* Uniforms (L3 reference): a mat4 product as the position and a varying copied from its own attribute (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec4 a_color; uniform mat4 u_mvp; varying vec4 v_color; void main(){ gl_Position = u_mvp * a_pos; v_color = a_color; }
