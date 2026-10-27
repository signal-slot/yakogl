/* Selector population: Whole position, a vec2 varying of another attribute (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; VsVarying comps 2). */
attribute vec4 p; attribute vec2 c; varying vec2 v; void main(){ gl_Position = p; v = c; }
