/* Textures (L3 reference): the position and a vec2 coordinate passed through (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; attribute vec2 a_uv; varying vec2 v_uv; void main() { v_uv = a_uv; gl_Position = a_pos; }
