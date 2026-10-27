/* Selector population: Xy01 position, a vec2 varying copying the position's own attribute (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; VsVarying attr == position attr). */
attribute vec2 pos; varying vec2 v; void main(){ gl_Position = vec4(pos, 0.0, 1.0); v = pos; }
