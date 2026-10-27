/* S4 ALU row: fsign (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; the program is highp fp32 on a varying, as femtovg's are). */
attribute vec4 p; attribute vec4 c; varying vec4 v; void main(){ gl_Position = p; v = c; }
