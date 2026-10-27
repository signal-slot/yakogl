/* Selector population: Whole position, a vec4 varying copying the position's own attribute (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; VsVarying attr == position attr). */
attribute vec4 p; varying vec4 v; void main(){ gl_Position = p; v = p; }
