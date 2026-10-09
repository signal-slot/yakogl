// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * gl_test.c -- YakoGL on silicon: the GL suite every board runs. The Zephyr
 * samples gl_test (PowerVR), gc_gl_test (GC7000) and v3d_gl_test (V3D) call
 * gl_test_run(); the host harness (tools/hostharness) runs it
 * on the Linux OSAL. (It began in the pvrlab kernel-module harness,
 * pvrgpu.ko, which is retired: README.md.)
 *
 * Bring the OSAL and the backend up, create a GL context and run one of the
 * cases through the public GL ES 2.0 API. Every case reads its result back
 * with glReadPixels, checks a few pixels against the values the maths
 * predicts, and prints "gl_test: case N <name>: PASS/FAIL" (plus the
 * dominant-channel map and the sampled pixels so a failure can be diagnosed
 * from the console alone).
 *
 * Cases (gl_test_run's gl_case):
 *   0 triangle  slice A: glClear + glDrawArrays of one Gouraud triangle with the
 *               color_uv.vert/frag pair (the V1+F4 shaders of the PVR_MULTIVARY
 *               test rig). pretransform=1 feeds screen-space vertices with the
 *               TA viewport transform off; pretransform=0 feeds NDC through
 *               the hardware viewport.
 *   1 uniform   slice B: fragment push-constant uniform (uniform_color.frag)
 *   2 texture   slice C: 2x2 nearest texture on a full-screen quad (texture.frag)
 *   3 depth     slice D: depth test LESS with three overlapping triangles
 *   4 cull      slice D: glCullFace / glFrontFace
 *   5 scissor   slice D: glScissor
 *   6 fbo       slice E: render to a texture through an FBO, then sample it
 *   7 all       the regression set: the `order` list in gl_test_main (40 cases
 *               with 172 and 173; 17 and 18 run last because they take over
 *               the context pool)
 *   8 blend     glBlendFunc over / additive / premultiplied (FS blend variants)
 *   9 stencil   mask/test passes, INCR, glClearStencil, glStencilMask, glColorMask
 *  10 prims     TRIANGLE_FAN from client-side arrays, u8/u32 indices, LINE_LOOP with
 *               glLineWidth, LINES, LINE_STRIP, POINTS
 *  11 persist   colour kept across kicks: a frame without glClear, and 300 draws
 *               in one frame (split by the backend)
 *  12 stencil2  two-sided stencil (glStencil*Separate), both windings, window + FBO
 *  13 zls       depth/stencil kept across kicks (ISP Z load/store), glDepthRangef
 *  14 api       the remaining entry points: constant attributes, glTexSubImage2D,
 *               glCopyTex*Image2D, queries, glIs*
 *  15 egl       EGL: pbuffer surface + queries
 *  16 msaa      4x MSAA EGL config (EGL_SAMPLES 4), PBE resolve
 *  17 ctx2      two GL contexts (GL_MAX_CONTEXTS)
 *  18 image     EGLImage (EGL_KHR_image_base / gl_texture_2D_image / pixmap)
 *  19 es100     GLSL ES 1.00 sources (the PowerVR SDK HelloAPI shaders verbatim)
 *  20 etc       ETC1 / ETC2 compressed textures
 *  21 rgb565    RGB565 window surface (PBE R5G6B5 pack, TPU R5G6B5 read-back)
 *  22 vao       vertex array objects (GL_OES_vertex_array_object; ES3 core)
 *  23 winding   nonzero-winding stencil-then-cover path fill with a hole (FemtoVG)
 *  24 compact   sampler compaction: a used non-zero binding lands at sh0
 *  25 stress    continuous-render stress, 8 tris/kick x 6000 kicks (short geom)
 *  26 heavy     long geom phase per kick (thousands of prims) to force a
 *               mid-render VDM context store -- reproduce the tiler stall
 *  27 fvgrepro  FemtoVG "invisible fill": link femtovg/main.vert (position at
 *               vi_reg 2, not 0) + fill_color.frag and read back a green fill
 *  28..33       the bisection of case 27 (main.vert + fvgtest_*.frag), kept as
 *               regression tests: 28 fvgvs (VS alone, trivial green FS), 29 fvgft
 *               (ftcoord varying), 30 fvgin (the frag[14] uniform block), 31 fvgcov
 *               (coverage terms, no discard), 32 fvgdis (same + a never-firing
 *               discard), 33 fvgdis2 (a never-taken texkill must not kill fragments)
 *  34 unishare  two draws of one program with different u_color in one kick:
 *               each draw must keep its own uniform (the per-draw snapshot; the
 *               bug the Slint/FemtoVG UI hit -- one shader, one colour per draw)
 *  35 tex1d     a 256x1 RGBA texture (femtovg's multi-stop gradient look-up:
 *               glTexImage2D NULL + glTexSubImage2D, REPEAT on T, linear)
 *  36 fvgconic  femtovg's conic gradient shaders (two-colour and look-up-texture
 *               variants) with main.vert: the angle picks the colour
 *  37 fvgstroke a gradient paint on femtovg stroke geometry (fringe tcoords +
 *               strokeMult) vs the same paint on fills
 *  38 fvgstencil femtovg's StencilStroke three-pass sequence (discard firing
 *               in the stencil INCR pass) with a texture look-up FS
 *  39 perf      wall time and GPU busy cycles per frame for 11 probes into a
 *               1024x600 FBO: clear only, blended / unblended / punch-through /
 *               ISP-only overdraw, 256 draws vs one draw (log only, always passes)
 *
 * **The numbering has gaps.** 7 is the regression set rather than a case, 50
 * is unused, and more will appear as cases are retired. A tool that sweeps a
 * range must not count a gap as a failure: run_case() returns -2 for "there is
 * no such case", distinct from -1 for one that ran and failed.
 *
 * The pixel values are AABBGGRR as stored (little-endian RGBA8888).
 *  58 clearscis glClear honours the scissor box, and lands where GL put it
 *  59 clearmask glClear honours the colour, depth and stencil write masks
 *  60 wrapmode  GL_REPEAT / GL_MIRRORED_REPEAT / GL_CLAMP_TO_EDGE, per axis
 *  61 texincomp GL's incomplete texture samples (0,0,0,1)
 *  66 clearvp   glClear is not bounded by the viewport (ES 2.0 3.7.3)
 *  68 armselect one arm of an `if` writes one local and the other writes
 *               another: both sides of the condition are drawn and read back.
 *               The in-process compiler selected an UNWRITTEN register at the
 *               join and was wrong on one side (gles2-core fce851e, in 30 of
 *               the 137 sources that compile -- the FemtoVG set). No other
 *               case has a branch at all, which is why 25/29 stayed 25/29
 *               through every board run while those shaders miscompiled.
 *               Not in the regression set until the offline table is rebuilt
 *               with its shaders (yakogl task 39)
 *  69 ioswap    the colour attribute declared before the position: the varying
 *               belongs in the register the position arrives in. The in-process
 *               compiler wrote it there first and then read that register as
 *               the position (40 of the corpus's vertex shaders, every Qt Quick
 *               one with a texture coordinate). Online-path only, like 68
 *  70 loopcarry a loop that reads, on every trip, a bound and an addend made
 *               before it; the in-process allocator gave their registers to a
 *               value made later in the body (femtovg's blur, filter_image.frag).
 *               Online-path only, like 68
 *  71 deadloop  work nobody reads before a loop and inside it; the in-process
 *               back end deletes it, which renumbers the break, the back edge
 *               and the ABI's back-edge list. Online-path only, like 68
 *  72 intlit    integer literals are integers: an int-counted loop (3 passes),
 *               an int compare against a literal read back on both sides, and
 *               ivec2/bvec2 constructors of constants, one colour channel each.
 *               The in-process compiler lowered every int literal as a float
 *               constant until 2026-09-23 (the loop ran twice, the compare was
 *               never true: FemtoVG's `texType == 1/2` branches were dead).
 *               Not in the regression set until the offline table is rebuilt
 *               with its shader (yakogl task 39)
 *  73 branchlane a discard behind a test of lane y and one behind lane w; the
 *               in-process compiler fed a branch its condition as .xyzw, so
 *               lanes nobody computed took part in the test (the reference
 *               splats one lane in all 208 of its branches). Online-path only
 *  74 ifoutput  gl_FragColor written inside an `if` and never read: in both
 *               arms, and before an if-only arm. If-conversion joined slots
 *               but not outputs, so both stores ran and the later one won
 *               whatever the condition said. Online-path only
 *  75 immneg    literals subtracted, in both stages: the in-process compiler
 *               negated an immediate by toggling a payload bit, so -1.0 was
 *               5.4e-20 -- femtovg's main.vert put every draw half a viewport
 *               to the right. Online-path only
 *  76 boolany   a bool and a bvec3 uniform (the GL core stores true as 1, the
 *               IR as all ones) and any()/all() of them and of a greaterThan()
 *               whose deciding lanes are not x; four sides, colours from the
 *               IR evaluator (lower.rs gl_test_boolany_*). Online only
 *  77 transc    sin/cos/log2/exp2 at exact arguments, and scalar operations on
 *               vectors: the in-process compiler emitted a plain sin/cos/log (this
 *               core's take pi-units and write a lane pair to be multiplied) and
 *               one scalar instruction per vector (lane x broadcast). Online only
 *  78 matvary   a mat2 varying (a register per column) and a varying after it;
 *               the in-process compiler refused every matrix varying. Online only
 *  79 idiv      integer division, truncated toward zero, both signs, vector
 *               lanes; refused as Lower(Unsupported(Idiv)). Online only
 *  80 matdyn    matrix arithmetic on a non-symmetric mat2 (m+m, k-m, k/m, -m,
 *               m++, m*=m), a vector ?: drawn on both sides of its condition,
 *               and dynamic indexing of a local array (read and write), a
 *               matrix column and vector lanes; colours from the IR evaluator
 *               (lower.rs gl_test_matdyn_*). Online only; both code
 *               generators build it (host-checked at 63b50b3's merge)
 *  90 churn     1,000 distinct programs created, compiled, linked, drawn once
 *               and deleted in one process, with the in-process compiler's keep
 *               region read before, after 200 and at the end: the free path
 *               (d13862d) must give it all back. Before it, every compile was
 *               refused after about 200 programs. Online only; the decisive
 *               run is on Zephyr, where that region is the only memory
 *  81 branchquad two discards behind tests of lanes y and w of a varying,
 *               so every quadrant takes its own branches; 73 drew its
 *               conditions from a uniform and could not tell a branch that
 *               read the wrong lane on silicon. Online only
 *  84 fragcoord gl_FragCoord.xy into the colour, on the default framebuffer
 *               (stored top-down, drawn with flip_y) and on an FBO: y is
 *               bottom-up in both, and the same pixel reads the same value
 *  86 anyall   all(equal(vec2 attribute, 0)) and any(greaterThan(vec3
 *               attribute, 0.5)): the reductions fold four lanes, and folded
 *               without repeating a real lane into the spare ones they read
 *               the vertex fetch's padding (0, 1). Online only
 *  87 databreak a loop whose break point differs from pixel to pixel and a
 *               continue taken in half of them: the online PowerVR path keeps
 *               it as one loop with a backward branch on a counter every
 *               instance shares, the exits as flags. Eight bands by two
 *               halves read back. Online only
 *  88 rsflip   the flip study's arm (ii), GC7000 only, with the test knobs: a
 *               1280x800 FBO frame timed with and without 100 RS_CONFIG_FLIP
 *               copies, and read back to see whether the copies reversed the
 *               rows. A measurement
 *  89 flatlast a flat vec4 and a flat uint across one triangle whose three
 *               vertices differ: every pixel takes the LAST vertex's values (GL's
 *               provoking vertex). GLSL ES 3.00
 *  97 varyings15 fifteen vec4 varyings, each a distinct constant, checked in the
 *               fragment shader: green when all arrive, else blue names the first
 *               wrong one. GL_MAX_VARYING_VECTORS 15 on the GC7000; skipped by
 *               name on a GPU that reports fewer
 * 101 cubebias  a 64x64 cube map, one colour per level: textureCube with bias
 *               0/+1/+2 at one texel per pixel must read levels 0/1/2 (texldb),
 *               textureCubeLod 0..3 in the vertex stage levels 0..3 (texldl)
 * 105 pointcoordvary gl_PointCoord after a varying (case 83's sprite, plus the
 *               vertex colour's blue): its components start at 4, not 0
 * 116 uniidxe  108 D's fragment shader under a vertex shader with uniforms and
 *               its own run-time-indexed read (gl_test 100 idxtex's VS shape)
 * 117 uniidxf  108 D's fragment shader under a vertex shader with a uniform
 *               and no run-time index
 * 120 uniidxg  108 D's fcols[fidx] plus a texel, the texld selected between
 *               the movar and the relative read (idxtex's order)
 * 121 uniidxh  the same with the texld after the relative read
 * 122 uniidxi  the same with the texld before the movar
 * 106 mrt3     three draw buffers: a clear reaching all three, one draw writing
 *               each its own colour, and GL_NONE in the middle leaving it alone;
 *               SKIP below GL_MAX_DRAW_BUFFERS 3
 * 128 uniidxj  108 D's fcols[fidx] in a fragment stage that also reads a
 *               varying (idxtex's shape): is a fragment stage with no varying
 *               input the class that reads element 0?
 * 127 lines    where a line's varyings are interpolated: two red-to-green lines
 *               with fractional endpoints, every covered pixel fitted against
 *               dEQP's model, the covered span's, and the pixel corner's
 * 113 cubemap   a 16x16 cube with 5 levels whose every texel encodes its face,
 *               level and (x, y); textureCubeLod aimed at one texel of each
 *               face at each level, the fetched (face, level, x, y) decoded:
 *               where the texture unit reads each face of each level
 * 108 uniidx    uniform arrays read at a run-time index, four programs: dEQP's
 *               float_dynamic_read (four reads, each right after its movar),
 *               one read right after its movar, one read five instructions
 *               after it, and idxtex's vec4 shape in ES 1.00. Online only
 * 123 zsapart   dEQP fbo.render.shared_depthbuffer and recreate_stencilbuffer
 *               (no_rebind) replayed, each from no kept depth/stencil buffer,
 *               with depth and stencil in one buffer and apart (v3d_split_zs /
 *               pvr_split_zs): shared must draw the FB2 picture (the depth
 *               lost) in one buffer and the right one apart; recreate must draw
 *               right apart (in one buffer it is printed, not judged). Every
 *               pixel off the depth-tie diagonal. V3D and PowerVR
 * 124 binovf    the binner's overflow blocks: frames of N checkerboard draws on
 *               a 1024x1024 FBO, N growing by half, until one job's tile lists
 *               outgrow the pool; that frame must use an overflow block and draw
 *               right (every pixel). With no blocks configured (the control
 *               image), the frame fails by name, counted once; run it last.
 *               V3D only
 * 125 texsubfbo dEQP fbo.render.texsubimage (after/between_render, rgb/rgba)
 *               replayed as dEQP draws it: an RGB source LINEAR/REPEAT into a
 *               128x128 fbo texture, a gradient sub-image at (32, 32), the
 *               blended second render, then the texture LINEAR on the window;
 *               the texture and the window judged whole against a C reference,
 *               with the mean difference per channel and the box of differing
 *               pixels. dEQP's window clear comes first: the render into the
 *               texture then ends the clear's job (the FB2 failures). V3D only
 * 126 etcmip    an ETC1 mip chain: 16x16, five levels each one flat colour,
 *               NEAREST_MIPMAP_NEAREST, the quad drawn into a 16>>l viewport
 *               so the texture unit picks level l; every pixel of each
 *               viewport judged. The PowerVR read level 0 only, and its
 *               descriptor put the level count in the twiddled word's
 *               baselevel (2026-09-24)
 * 129 vstexlod  a VS texture2DLod on a 16x16 GL_RGB mip chain (texels encode
 *               level and x, y), lightest pass first, each its own job: a
 *               look-up with a constant coordinate and lod; a one-level
 *               control; the chain NEAREST at lod 0, 1, 2; mipmapped
 *               at lod 0, 1, 2; lod -1.74 both ways; then a second GL_RGB chain
 *               on unit 1 sampled by the FS; last, dEQP texture.vertex.2d's
 *               own draw (a 127x127 grid, 1.2 MB of client arrays). Every
 *               point and the black between checked; the first wrong point
 *               printed with levels 0..2 of both textures there (dEQP
 *               texture.vertex.2d). V3D and GC7000
 * 149 vsunits   sixteen vertex-stage samplers in one program: sampler k reads
 *               a 1x1 texture of its own colour on unit k, and draw k (a
 *               one-hot uniform weight) puts sampler k's colour in cell k
 *               of a 4x4 grid; every cell's centre judged. dEQP-GLES2
 *               uniform_api wants 9..14 vertex units
 * 150 shprobe   which vertex shared registers a shader reads: a uniform
 *               array of colours that name their own index, read cell by
 *               cell through one-hot selectors below sh256; arm A sh0..255,
 *               arm B sh0..279. Not in the default set (a probe)
 * 166 fshprobe  case 150 in the fragment stage (arms A, B), and arm C: 16
 *               fragment samplers (descriptors sh0..319, 13..15 past sh255)
 *               with one-hot weights at sh320..335, sampler k's colour in
 *               cell k. Not in the default set (a probe)
 * 130 allfeat3  dEQP shaders.random.all_features.fragment.3 (rsg seed 3): its
 *               constant-colour FS with no varyings, drawn as glsRandomShaderCase
 *               does (client arrays, glDrawElements, no clear), on the window
 *               and a 256x256 FBO at (119, 7); controls: the colour as a
 *               literal, and through a varying. Per channel against (255, 207,
 *               143, 199). V3D only
 * 133 texcubemip cube-map mipmaps against dEQP's reference (its cube LOD
 *               bounds): a 64x64 cube, 7 levels, one colour per face and
 *               level; per face NEAREST_MIPMAP_NEAREST at LODs allowing level
 *               0..4 and LINEAR_MIPMAP_LINEAR blending 1-2 and 2-3, face
 *               interiors only; each draw names the (face, level) it reads.
 *               The scene and the verdict are tools/v3d/tex_cubemip_case.h.
 *               V3D only for now
 * 134 texcubethin dEQP's thin cube-mipmap cells (17 and 7 wide, and turned)
 *               on every face, the direction as interpolated and divided by
 *               its major axis; tools/v3d/tex_cubethin_case.h. V3D only
 * 135 texnarrow L8, A8 and LA8 sampled narrow (R8 / RG8 with a swizzle): a
 *               64x64 chain level by level, a 13x7 level and a 16x16 cube at
 *               levels 0 and 2 on every face, every pixel against its texel's
 *               hashed bytes; tools/v3d/tex_narrow_case.h. V3D only
 * 136 psizmax   where the V3D clamps gl_PointSize (dEQP
 *               rasterization.limits.points): one red point per size (64 to
 *               1024) in a 1280x4 FBO, its width on one row; the verdict is
 *               width s up to 512 and 512 above (Mesa's V3D_MAX_POINT_SIZE),
 *               every width printed. V3D only
 * 137 psizmin   gl_PointSize below 1.0: one red point per size (0.25, 0.5,
 *               0.99, 1.0, 1.5) on a pixel centre of a 16x16 FBO; each must
 *               draw exactly that pixel (GL clamps to [1, max]); the count and
 *               box printed, or NOTHING drawn (Mesa 3539bd63: the hardware
 *               draws nothing below 1.0). V3D only
 * 138 deriv     dFdx / dFdy of a varying and of gl_FragCoord (es100/deriv),
 *               each 0.5 + the derivative scaled to 0.25, on the window
 *               (y scale -1) and on an FBO (+1): 0xffbfbfbf everywhere on
 *               both; a dFdy sign error reads green 64 on one of them.
 *               Written for the V3D; other GPUs give what their paths give
 * 140 tfuprobe  the TFU (never used by the product) against our CPU tiling:
 *               RGBA8/RG8 (both job types) and R8 at odd widths through LT,
 *               UBLINEAR, UIF and UIF XOR, six source strides/addresses,
 *               byte-compared at every texel; plus 1920x720 RGBA8 timed
 *               against the CPU. Verdict: Mesa's shape (64-byte stride,
 *               page-aligned source) equal everywhere, no GPU reset; the
 *               other variants are findings (tools/v3d/tfu_case.h). Opt-in.
 *               V3D only
 * 141 es3instdiv  case 47's program with glVertexAttribDivisor 2, 4 instances,
 *               arrays and elements: red bottom left (instances 0-1), half
 *               green bottom right (instance 3 last, gl_InstanceID == 3),
 *               the top clear. Passes where the backend takes the divisor
 *               (the V3D, with the P1/P2 proposal); elsewhere the GL core
 *               draws per instance and gl_InstanceID needs P2
 * 145 samplecov glSampleCoverage on a 4x pbuffer (EGL_SAMPLES 4): red at
 *               coverage 0.5 then green at 0.5 inverted (each half of the
 *               mask, resolved 0x80 each), red at 0.25 (0x40), at 0.0
 *               (black) and with coverage off (full red), read back at
 *               three pixels each. SKIPs where the build offers no 4x
 *               config (the V3D's v3d_debug_msaa, the GC7000's
 *               gc_debug_msaa off or not built in). Written for the V3D
 *               (MSAA step 3); the PowerVR offers 4x but no backend there
 *               takes the mask
 * 148 texbranch look-ups inside uniform branches (es100/texbranch.frag): a
 *               two-armed `if` sampling texture A or B (red), a one-armed
 *               `if` with a look-up (green, 0 when not taken) and femtovg
 *               filter_image's loop (24 passes, a uniform break, two
 *               look-ups a pass: blue, 1/48 of B's blue each), drawn three
 *               times (then arm, gate on, 3 passes; else arm, gate off, no
 *               pass; else arm, gate on, all 24): the colour computed on
 *               the CPU, read back at three pixels. On the V3D every `if`
 *               is a uniform branch and an untaken arm's look-ups are not
 *               requested. Written for the V3D; other GPUs give what their
 *               paths give
 * 142 unibranch a large `if` on a uniform (es100/unibranch.frag), drawn with
 *               the condition true and false: each arm's colour, computed
 *               on the CPU with the same formula (the two differ in every
 *               channel). On the V3D a branch round the untaken arm. Written
 *               for the V3D; other GPUs give what their paths give
 * 162 dskeep    framebuffers with a depth renderbuffer each, depth test on,
 *               until a draw is GL_OUT_OF_MEMORY (by the 33rd on the V3D,
 *               which keeps 32 depth/stencil buffers, by the ninth on the
 *               PowerVR and the GC7000, which keep eight; the window's
 *               among them); framebuffer 0's depth is read back intact
 *               through a depth-tested quad (blue below the diagonal, its
 *               green above), and once its renderbuffer is deleted the
 *               refused framebuffer draws. All three GPUs
 * 167 stcycle   femtovg's layer pattern: 13 framebuffers (7 on the PowerVR
 *               and the GC7000, which keep eight depth/stencil buffers),
 *               each a colour texture and a STENCIL_INDEX8 renderbuffer
 *               only, cleared with a scissor over the whole image, then a
 *               stencil pass (INCR) over a triangle and a red cover pass
 *               (stencil NOTEQUAL 0). No draw may be GL_OUT_OF_MEMORY. Then
 *               each framebuffer again, stencil EQUAL 1 and no clear: green
 *               lands in its own triangle only (lower left for even, upper
 *               right for odd), so each kept its own stencil. All three GPUs
 * 168 stlayers  167 with the cluster's numbers: 13 stencil-only layers of
 *               600x600 alive together, each drawn as in 167, then each
 *               drawn again (stencil EQUAL 1) and two pixels read back from
 *               the layer. Prints how many were refused (GL_OUT_OF_MEMORY):
 *               a measurement (13 - 8 expected on the PowerVR and the
 *               GC7000, 0 on the V3D), not a pass condition; passes when
 *               every error is that refusal and every layer drawn first
 *               draws again and reads back its own stencil. All three GPUs
 * 169 dstex     depth textures (GL_OES_depth_texture, GL_OES_packed_depth_stencil;
 *               ES 3.0 core): (1) a 4x1 DEPTH_COMPONENT/UNSIGNED_SHORT ramp
 *               uploaded and sampled, (d, d, d, 1) on ES 2.0, (d, 0, 0, 1) on
 *               ES 3.0; (2) one DEPTH_STENCIL/UNSIGNED_INT_24_8 texture on the
 *               depth and stencil points of an FBO, drawn with both tests
 *               (green only where both pass); (3) that texture sampled: the
 *               depth the draws wrote. SKIP on an ES 2.0 library without
 *               the two extensions. The V3D samples the R32F texels at the
 *               TMU's 32-bit output (LodKey::ret32) and resolves its D24S8
 *               depth into them (v3d_depth_resolve); PASS on the Pi 4,
 *               2026-10-04 (tools/cts/RESULTS.md).
 *               PowerVR, GC7000 and V3D
 * 177 zonly     a framebuffer whose only attachment is a depth texture
 *               (glmark2's shadow pass, scene-shadow.cpp:100-118: a
 *               DEPTH_COMPONENT/UNSIGNED_INT texture at twice the window):
 *               the status, a depth ramp drawn into it with no colour
 *               buffer, then the texture sampled over the window: the depth
 *               the draw wrote. Complete where the backend draws without a
 *               colour target, which all three GPUs do now: the GC7000; the
 *               V3D, a job with no render target loaded or stored, PASS on
 *               the Pi 4 2026-10-04; the PowerVR, a z-only render as in
 *               Mesa's pvr, PASS on the AM62P 2026-10-04.
 *               GL_FRAMEBUFFER_UNSUPPORTED is a SKIP by name, as is an ES 2.0
 *               library without GL_OES_depth_texture
 * 178 heldfree  the previous scene's teardown, then shadow's 1600x1200 depth
 *               texture at once: the GPU pool filled with 1 MiB buffers and
 *               all deleted with no frame open, then case 177's steps at
 *               1600x1200. Without the reclaim of held frees the texture is
 *               refused (GL_OUT_OF_MEMORY); PASS on the AM62P with the
 *               reclaim (2026-10-04). Not measured on the V3D or GC7000 yet
 * 179 retile    a depth texture rendered and sampled in two frames with no
 *               glFinish between (glmark2's shadow): each frame's depth
 *               read back through the texture (0x40, then 0xbf). On the
 *               V3D also the carve-out in use after each sampling draw,
 *               its job open: equal in both frames (the old stamp's tiled
 *               copy given back before the job opened, not held for it).
 *               All three GPUs; on the Pi 4 PASS with b7170d35 and
 *               FAIL at check 8 (+266240 bytes) without it (2026-10-04)
 * 180 teardown  a 1024x1024 texture sampled, the job flushed and left to
 *               finish unobserved, the texture deleted and one of the same
 *               size made: both read back (red, then green); on the V3D
 *               also the carve-out does not grow (the finished job's frees
 *               come back at once, v3d_frees_poll). All three GPUs;
 *               on the Pi 4 PASS with a528c8a3 and FAIL at check 5
 *               (+5592576 bytes) without it (2026-10-04)
 * 172 swapzs    a window's depth across eglSwapBuffers (EGL 1.5 3.10.1:
 *               undefined after the swap): frame 1 red at depth 0.25 over the
 *               left half, swap, frame 2 green at 0.5 LESS with no depth
 *               clear. Asserts each backend's defined behaviour: left black
 *               where the depth survives (the GC7000; the PowerVR with
 *               CONFIG_GL_TEST_PVR_KEEP_ZS), green where the swap drops it
 *               (the V3D; the PowerVR by default). All three GPUs; in the set
 * 173 splitzs   a window frame split by glFinish keeps its depth: red at 0.25
 *               on the left, glFinish, green at 0.5 LESS: left stays red,
 *               before and after a swap. All three GPUs; in the set
 * 181 fusemad   a * b + c as one fmad (the PowerVR's fuse_mul_add): x - 2.0 * q
 *               (a negated immediate factor), x - a * b, 2.0 * q + b, and the
 *               controls it must leave alone (a product read twice, under
 *               abs, under floor), each in a fragment and a vertex shader,
 *               from uniforms, read back. All three GPUs (skipped by name in a
 *               table build: no rows for its own sources); in the set
 * 182 fsat      clamp(x, 0.0, 1.0) as one saturating add (the PowerVR's fsat,
 *               prepare.rs fusion_pairs): clamp(), min(max()) and max(min())
 *               of x below 0, inside and above 1, and the controls it must
 *               leave alone (the inner bound read twice, an upper bound of
 *               0.8, a lower bound of 0.2), each in a fragment and a vertex
 *               shader, from uniforms, through * 0.5 + 0.25 so a missing
 *               saturate shows past the RGBA8 write's own clamp, read back.
 *               All three GPUs (skipped by name in a table build); in the set
 * 184 depchain  the PowerVR's cost of a back-to-back dependency: 48 fmads as one
 *               dependent chain, as two interleaved chains and as four (the
 *               online compile keeps the order: 49, 48 and 49 groups at
 *               dependency distance 1, 2 and 4), 16 full-screen quads into
 *               a 1024x600 target each, 3D busy logged per arm, the colour
 *               read back. A measurement (like 39): the arms decide whether
 *               the online fill_color's 56 back-to-back pairs (the table's
 *               48) are worth a scheduler. All three GPUs; not in the set
 * 188 predif    an `if`/else whose arms are arithmetic (the PowerVR runs them
 *               predicated, as pco does: pco_cf.c lower_if_pred_exec), each
 *               side read back: on a uniform in the fragment shader and in
 *               the vertex shader, each drawn with the condition true and
 *               false, and on gl_FragCoord.x, so instances of one draw take
 *               different arms. All three GPUs (skipped by name in a table
 *               build); in the set
 * 189 mulone    a product by the constant 1.0 (the PowerVR compile reads x
 *               for `x * 1.0`, no group): a uniform mat3 times
 *               vec3(0.5, 0.5, 1.0), a negated uniform among its columns;
 *               the same matrix times vec3(0.5) (no lane is 1.0: every
 *               product is kept); abs() and a negation multiplied by 1.0.
 *               Each read back. All three GPUs (skipped by name in a table
 *               build); in the set
 * 190 blendonce a blend factor that is the same in every channel (1 - As,
 *               1 - Ad) is computed once and read by the later channels on
 *               the PowerVR, as pco's variants do. Over a pixel that is
 *               non-zero in every channel: every channel sharing it, red
 *               masked (green computes it), MIN on the colour (alpha
 *               computes it alone), and both factors shared at once. Each
 *               read back. All three GPUs (skipped by name in a table
 *               build); in the set
 * 191 varyfma   products of a varying fused with their sum (the PowerVR
 *               compile makes `v.x * 2.0 - 1.0` one fmad, as pco does; the
 *               varying sits in the temporary its iteration pinned):
 *               v.x * 2 - 1, v.y * 0.5 + 0.25, 1 - v.x * v.y, read back.
 *               All three GPUs (skipped by name in a table build); in the
 *               set
 * 192 predlive  values live across a predicated `if` (the PowerVR writes
 *               an arm-computed value to one register from both arms, as
 *               pco does, with no select after): two values from before
 *               the `if`, read in both arms and after it, beside two the
 *               arms compute; on a uniform each way and on gl_FragCoord.x
 *               either side of 32, read back. All three GPUs (skipped by
 *               name in a table build); in the set
 * 193 offarm    does a predicated-off group cost time? 186's arm 1
 *               (fill_color.nodiscard, femtovg's blend, scissorRadius 0) with
 *               the scissor arm radius 0 never runs grown by N = 0, 8, 32 and
 *               128 dependent steps (tools/shaders/offarm/), 3D busy logged per
 *               N, every N the same picture. Run in an online image and in a
 *               table image. A measurement; all three GPUs, not in the set
 * 194 offarmn   193 over 14 N: 0, 2, 4, 8, 12, 16, 24, 32, 48, 64, 96, 128,
 *               192 and 256 dependent steps in the arm radius 0 never runs,
 *               3D busy logged per N, every N the same picture. Run in an
 *               online image and in a table image. A measurement; all three
 *               GPUs, not in the set
 * 195 ifbranch  what does a branch over an `if` arm cost? 193's programs
 *               (N = 0, 8, 32, 128 steps in the arm radius 0 never runs),
 *               each compiled with its arms predicated and with a branch
 *               past each arm (PVR_BACKEND_OPT_IF_LOWERING) on one context,
 *               3D busy logged per program and form, every one the same
 *               picture. The run-time compiler only (skipped by name in a
 *               table build); all three GPUs, the option the PowerVR's. A
 *               measurement, not in the set
 * 197 vtxgrid   dEQP texture.vertex.2d's draw: its 127x127 grid of client
 *               arrays (1,225,804 bytes with the indices) drawn four times,
 *               one viewport quarter each, the VS sampling a mipmapped
 *               16x16 GL_RGB chain with texture2DLod at lod 0, 1, 2, 3;
 *               every pixel read back against its level's texel. The
 *               PowerVR refused each such draw with its 1 MiB GENERAL arena
 *               (the 45 texture.vertex.2d cases at 32778b57) until indexed
 *               draws were cut into pieces. All three GPUs (skipped by
 *               name in a table build); in the set
 * 187 predarm   186 with scissorRadius 4: fill_color's other scissor arm. The
 *               table predicates the two arms (32 and 22 groups) and ours
 *               computes both; the table should cost about 10 groups more
 *               than at radius 0, ours the same. A measurement; all three
 *               GPUs, not in the set
 * 186 fcsplit   case 39's fill_color split by feature: fill_color and its twin
 *               without the discard (femtovg/fill_color.nodiscard.frag),
 *               each with femtovg's blend and with blend off, 16 full-screen
 *               quads into 1024x600 per arm, 3D busy logged; each pair must
 *               draw the same picture. Run in an online image and in a table
 *               image, the ratio per arm says which feature carries the
 *               online program's 22% (184 and 185: not the group count, not
 *               the kind). A measurement; all three GPUs, not in the set
 * 185 kindcost  the PowerVR's cost per instruction group by kind, the next
 *               fill_color measurement after 184: arms of one kind each at
 *               fixed counts (host count in the case's comment): fmul,
 *               fmul + tst/movc pairs, fmul + mbyp moves, tst/movc + mbyp,
 *               fadd, and fmad with femtovg's blend (the ordered tail). 16
 *               full-screen quads into 1024x600 per arm, 3D busy logged,
 *               the colour read back. A measurement (like 39 and 184); all
 *               three GPUs, not in the set
 * 183 unipad    500 draws of a program whose two stages each read a vec2
 *               uniform (femtovg's viewSize; the constant file's last register
 *               half empty), timed: under 250 ms, and the colour read back.
 *               The i.MX8MP cluster's frame 2 spent 2.7 s in the GC7000
 *               command stream this way, online only (2026-10-05): every draw
 *               logged the padding dwords as missing uniforms. All three GPUs
 *               (skipped by name in a table build); in the set
 * 176 mapbuf    GL_OES_mapbuffer: the slice-A triangle drawn from a VBO,
 *               then glMapBufferOES(WRITE_ONLY_OES): written nothing (the
 *               triangle is still there: unwritten bytes keep their
 *               contents), collapsed to one point (nothing drawn), written
 *               back (the triangle again); the ACCESS / MAPPED / pointer
 *               state on the way. All three GPUs; not in the set yet
 * 131 allfeat1  dEQP shaders.random.all_features.fragment.1 (rsg seed 1): a
 *               texture2D at the constant (1.0, 0.5) on a 64x64 RGBA gradient,
 *               LINEAR, CLAMP, no varyings, as glsRandomShaderCase sets it up;
 *               controls: the look-up at the literal (1.0, 0.5) and at the
 *               centre. Window and a 256x256 FBO at (78, 15). V3D only
 * 118 texmip    mipmaps against dEQP's reference (its LOD bounds and level
 *               choice): a 64x64 chain of one colour per level, uploaded level by
 *               level and once by glGenerateMipmap, drawn at LOD 0.61 and 2.19
 *               under the four mipmap min filters, every pixel checked; the scene
 *               and the verdict are tools/v3d/tex_mip_case.h. V3D only for now
 * 115 texcube   a 4x4 cube map, each face its own blue, on both filters against
 *               dEQP's reference (selectCubeFace, projectToFace, sampleLinear2D):
 *               12 draws, one per face and filter; LINEAR edge pixels measured,
 *               not judged. The scene and the verdict are tools/v3d/
 *               tex_cube_case.h. V3D only for now
 * 111 points    GL_POINTS on the window and on an FBO: sizes 1, 3 and 5 from
 *               gl_PointSize, then a VS without it (the POINT_SIZE packet's
 *               1.0); every pixel of the frame checked. V3D only
 * 110 psizvpm   a measurement: a triangle and a line drawn by a VS that also
 *               writes gl_PointSize, against the same VS without it; the whole
 *               framebuffer must match. Answers whether the V3D record's point-
 *               size bit only describes the VPM layout for every primitive.
 *               Needs a compiler that keeps the word for triangles; says NOT
 *               EXERCISED otherwise. V3D only
 * 109 texlinear  LINEAR filtering against dEQP's own reference (sampleLinear2D):
 *               a 4x4 ramp on four quads, LINEAR CLAMP / LINEAR REPEAT / NEAREST /
 *               min NEAREST + mag LINEAR minified; the scene, the reference and the
 *               verdict are tools/v3d/tex_linear_case.h. V3D only for now
 * 112 affectand dEQP's operator.sequence.side_effects.affect_and verbatim, drawn
 *               for u_false false (green) and true (red): the if-converted
 *               `&&` read its condition from a register nothing wrote.
 *               Online-path only
 * 114 cubelod   a 16x16 cube, a colour per face and level, LINEAR: textureLod
 *               at levels 3, 4 and 6 across the whole +X face, per vertex and
 *               per pixel; every pixel must be the level's flat +X colour (a
 *               face clamp taken at level 0 blends in -X at high levels)
 * 102 maskclear colour clears under a partial glColorMask: full screen R+A,
 *               scissored G, and B fused into a draw; every pixel checked. The
 *               PowerVR fails until its clear shader has a masked variant
 *  91 hdrblend  two blends into an RGBA16F target cleared above 1.0, with a
 *               source above 1.0: Quick 3D's premultiplied SourceOver and the
 *               plain over, one per half, read back at a quarter scale. The
 *               online PowerVR path blends in the shader as pco does: the pixel
 *               read as two dwords of halves, no clamp, two dwords written.
 *               ES 1.00 sources; online only (the offline table has no
 *               RGBA16F blend row for them)
 *  98 uniloop   a loop bounded by a uniform (Qt Quick 3D's light loop), counts
 *               0, 1, 7 and 4097 in four quadrants: kept as one loop, and 4097
 *               stops at the online path's deliberate cap of 4096. ES 3.00;
 *               online only (no offline blob)
 * 104 divloop   loops whose trip counts differ from pixel to pixel (eight
 *               bands: 0, 1..6 passes and the 4096 cap; and a nested pair
 *               with break and continue at both levels), under the execution
 *               mask; ES 3.00, online only. Run alone first
 * 100 idxtex    a texture and a dynamically indexed uniform array in each
 *               stage: the indexed read addressed past the sampler
 *               descriptors (the online path read 20 dwords too low until
 *               18ae1599). ES 3.00; online only (no offline blob)
 *  62 clipz    which near/far clip convention the TA uses, swept over
 *               CR_PPP_CTRL bit 0 x the clip mode, so the board's own output
 *               states it (PowerVR only: the sweep needs that GPU's knobs;
 *               elsewhere it measures the default and says so). Asserts what
 *               this build ships with -- 0x203 and FRONT_REAR clip GL's
 *               -w <= z <= w. Not in the regression set
 *  63 matattr   a mat2/mat3/mat4 vertex attribute: the columns reach the right
 *               vertex inputs (the colour is the matrix's first row), and GL
 *               still calls it one attribute at the first of its locations.
 *               Not in the regression set until the offline table is rebuilt
 *               with its shaders (yakogl task 39)
 *  64 bgra      GL_EXT_texture_format_BGRA8888: the same 2x2 picture uploaded
 *               as GL_RGBA and as GL_BGRA_EXT reads back identically, so a
 *               swap in the wrong direction (red and blue exchanged) fails.
 *               Not in the regression set
 *  40 qtstrip   a Qt Quick merged strip (repeated-index joins, per-vertex z from
 *               a third attribute: 9 VS inputs vs 8 outputs), 24 vertices under
 *               the depth test: every quad must show (VDM_STATE5 unified size)
 * 165 ctxclear  a pending glClear stays with the context that issued it
 *               (review R-04): the harness context A clears red, a second C API
 *               context B draws a green left half and clears blue, back to A:
 *               B's right half is never red, A reads red, B reads blue. Needs a
 *               free context slot (SKIP otherwise). All three GPUs
 * 163 drawbuf0  glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (ES 3.0 4.2.1,
 *               review R-09): attachment 0 must keep its grey; attachment 1 is
 *               green when the GPU draws it, grey when the backend refuses the
 *               draw (GL_INVALID_OPERATION, today on all three). SKIP on an
 *               ES 2.0 library or below GL_MAX_DRAW_BUFFERS 2
 * 164 bufdel    glDeleteBuffers of the buffer an attribute reads (ES 2.0.25
 *               2.9, review R-24): the attribute's buffer binding reads 0, and a
 *               draw after glGenBuffers reused the name must not read the new
 *               buffer (refused, GL_INVALID_OPERATION): the frame keeps its
 *               green left half on black. All three GPUs
 * 171 viewwin   a window that is a view into a wider buffer (rows 96 pixels
 *               apart for a 64-wide window): drawn, loaded and read back at
 *               its pitch, the first and last rows, the padding untouched.
 *               All three GPUs
 */
#if defined(GL_TEST_HOST) || defined(GL_TEST_USER)
/* The hosted builds: tools/check.sh compiles this file on the host with
 * GL_TEST_HOST (the PowerVR arm) and with GL_TEST_USER (the GC7000 arm), so a
 * syntax error in either GPU's code fails the gate without a board. Both log
 * with printf and keep time with clock_gettime. */
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <time.h>
static uint32_t now_ms(void)
{
	struct timespec ts;

	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (uint32_t)(ts.tv_sec * 1000u + ts.tv_nsec / 1000000u);
}
#define pr_info printf
#define pr_err printf
#else
/* Zephyr (samples/zephyr/gl_test): the same source, printk for the log. */
#include <zephyr/sys/printk.h>
#include <zephyr/kernel.h>
static uint32_t now_ms(void) { return k_uptime_get_32(); }
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#define pr_info printk
#define pr_err printk
#endif

#include <GLES3/gl3.h>   /* ES 2.0 plus the ES 3.0 entry points cases 41/42 use */
#include <EGL/egl.h>
#define GL_GLEXT_PROTOTYPES 1
#include <GLES2/gl2ext.h>
#include "gl_internal.h"
#include "backend.h"
/*
 * The suite itself is GL-level and runs on either GPU; only the few lines
 * that start the backend are specific to one. The PowerVR half brings its
 * options, its debug hooks and a register base for the OSAL; the Vivante
 * driver maps its own registers from the device tree and has no options, so
 * that half needs nothing but the OSAL.
 */
/* Zephyr spells the choice CONFIG_YAKOGL_GPU_VIVANTE (Kconfig), CMake spells
 * it YAKOGL_GPU_VIVANTE (CMakeLists.txt). One GPU, two build systems.
 *
 * Three GPUs: GL_TEST_VIVANTE, GL_TEST_V3D (the Raspberry Pi 4, Zephyr only,
 * driver/samples/zephyr/v3d_gl_test) and GL_TEST_PVR. A site that calls the
 * PowerVR's knobs or counters is `#ifdef GL_TEST_PVR`; a site that leaves out
 * what one GPU lacks names that GPU. */
#if defined(CONFIG_YAKOGL_GPU_VIVANTE) || defined(YAKOGL_GPU_VIVANTE)
#define GL_TEST_VIVANTE 1
#elif defined(CONFIG_YAKOGL_GPU_V3D)
#define GL_TEST_V3D 1
/* The OSAL's init argument (struct osal_zephyr_v3d_init: the V3D's clock
 * switch through the firmware mailbox), which only the board's sample has. */
void *gl_test_v3d_osal_arg(void);
#else
#define GL_TEST_PVR 1
#include "pvr/pvr_backend.h"   /* PowerVR options and debug hooks */
#include "pvr_render.h"
#endif
#include "osal_internal.h"
/* The library's test knobs (the crate's `test-knobs` feature): a Zephyr
 * image that selects them has CONFIG_YAKOGL_TEST_KNOBS, a CMake or host
 * build passes YAKOGL_TEST_KNOBS. Every call of a knob is under
 * GL_TEST_KNOBS; without it the case runs the library's default arm only,
 * or skips when the knob is the case. */
#if defined(CONFIG_YAKOGL_TEST_KNOBS) || defined(YAKOGL_TEST_KNOBS)
#define GL_TEST_KNOBS 1
#endif

/*
 * Two things only one of the two GPUs has. Neither changes what a case
 * checks: the performance counters are a log line, and the primitive budget
 * splits a draw across PowerVR tiles -- without it the two halves of that
 * case draw the same thing and comparing the picture with itself is still a
 * real comparison, just a weaker one.
 */
#if defined(GL_TEST_VIVANTE) || defined(GL_TEST_V3D)
static inline void gl_test_perf_read(uint32_t *out) { memset(out, 0, 7 * sizeof(*out)); }
static inline void gl_test_prim_budget(unsigned int budget) { (void)budget; }
/* how the run-time compiler lowers an `if` with arithmetic arms: the
 * PowerVR's option only (case 195); elsewhere both forms are one program */
#define GL_TEST_IF_DEFAULT   0u
#define GL_TEST_IF_PREDICATE 1u
#define GL_TEST_IF_BRANCH    2u
static inline void gl_test_if_lowering(unsigned int v) { (void)v; }
#else
static inline void gl_test_perf_read(uint32_t *out) { pvr_render_perf_read(out); }
static inline void gl_test_prim_budget(unsigned int budget)
{
	pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, budget);
}
#define GL_TEST_IF_DEFAULT   PVR_IF_LOWERING_DEFAULT
#define GL_TEST_IF_PREDICATE PVR_IF_LOWERING_PREDICATE
#define GL_TEST_IF_BRANCH    PVR_IF_LOWERING_BRANCH
static inline void gl_test_if_lowering(unsigned int v)
{
	pvr_backend_set_option(PVR_BACKEND_OPT_IF_LOWERING, v);
}
#endif
/*
 * The shader set. The GLSL 450 set exists for one reader: the PowerVR offline
 * table, which knows a shader by the SHA-256 of its text and holds the 450
 * texts, not their ES 1.00 twins. Every compiler -- the online one on the
 * three GPUs -- is an ES compiler that refuses
 * "#version 450" at the version line, which says nothing about it. So a build
 * that compiles its shaders takes the ES 1.00 set: asked for (GL_TEST_ES100,
 * which the Linux user build.sh sets from the archive it links), or a Zephyr
 * image with the in-process compiler or with the precompiled shaders (made by
 * that compiler, of the ES 1.00 set: the list's gl_test_es100 app).
 */
#if defined(GL_TEST_ES100) || defined(CONFIG_YAKOGL_SHADERC_ONLINE) || defined(CONFIG_YAKOGL_PRECOMPILED_SHADERS)
#include "gl_test_shaders_es100.h"
#else
#include "gl_test_shaders.h"
#endif
#ifndef GL_TEST_SHADERS_SKIP_REASON
/* The default set serves every case. Defined here rather than in the
 * generated header so that header stays byte-identical: the AM62P's offline
 * table is matched to it by SHA-256. */
#define GL_TEST_SHADERS_SKIP_REASON(c) ((const char *)0)
#endif

/*
 * Why the shader set cannot serve case `c`, or NULL. The ES 1.00 set names
 * the cases whose sources are "#version 300 es"; they are outside an ES 2.0
 * compiler, but a library that reports ES 3.0 compiles them, so there they
 * run. Asked of the current context; with none, the set's answer stands.
 */
static const char *shader_skip_reason(int c)
{
	const char *why = GL_TEST_SHADERS_SKIP_REASON(c);
	const char *v;

#ifndef GL_TEST_ES100_SET
	/* the desktop set is a table build's, and the table has no rows for a
	 * case's own sources */
	if (c == 181 || c == 182 || c == 183 || c == 184 || c == 185 || c == 188 || c == 189 || c == 190 || c == 191 || c == 192 || c == 197)
		return "its own sources: a table build has no rows for them";
	/* one program per source in a table: nothing to lower two ways */
	if (c == 195)
		return "the if lowering is the run-time compiler's: a table build has one program per source";
#endif
	(void)c; /* the default set's reason ignores it */
	if (!why)
		return (const char *)0;
	v = (const char *)glGetString(GL_VERSION);
	if (v && strncmp(v, "OpenGL ES 2.", 12) != 0)
		return (const char *)0;
	return why;
}

#define TW 64u
#define TH 64u

/* The case run_case() is in, for a watchdog thread with no console. */
volatile int gl_test_current_case = -1;

/*
 * Which check inside the case failed first, for the same caller.
 *
 * A single case comes back from a board with no console as one bit: the hold
 * is PASS or it is FAIL. That is enough to see a fix land and not enough to
 * find one -- "case 2 fails" covers a 2x2 texture, a gradient, three mip
 * levels and two texture units alike. Every expect_*() is therefore numbered
 * within its case, and the first one that fails leaves its number here, so a
 * run says which of them it was. The numbering is the order the checks run
 * in; inserting a check renumbers the ones after it.
 */
/*
 * Cases the caller wants left out, as a decimal list ("3" or "3,13"). A case
 * that hangs a kick costs every later case the two seconds each hung kick
 * waits, so one unfixed case can stop the set from finishing at all and hide
 * the state of the thirty that follow it. Skipping it is how the rest stay
 * readable while it is being worked on; a skipped case counts as neither a
 * pass nor a failure.
 */
const char *gl_test_skip;

static bool case_skipped(int c)
{
	const char *p = gl_test_skip;

	while (p && *p) {
		int v = 0;
		bool digits = false;

		while (*p == ' ' || *p == ',')
			p++;
		while (*p >= '0' && *p <= '9') {
			v = v * 10 + (*p++ - '0');
			digits = true;
		}
		if (digits && v == c)
			return true;
		if (!digits && *p)
			p++;   /* skip anything that is not a number */
	}
	return false;
}

volatile int gl_test_check_no;         /* checks made so far in this case */
volatile int gl_test_first_bad_check = -1;
/* What that check read, and what it wanted. "Check 23 failed" says where to
 * look; "check 23 read green where it wanted blue" often says why. */
volatile uint32_t gl_test_first_bad_got, gl_test_first_bad_want;

static bool gl_test_check(bool ok, uint32_t got, uint32_t want)
{
	gl_test_check_no++;
	if (!ok && gl_test_first_bad_check < 0) {
		gl_test_first_bad_check = gl_test_check_no;
		gl_test_first_bad_got = got;
		gl_test_first_bad_want = want;
	}
	return ok;
}

/* glReadPixels destination + the top-down copy the checks index (row 0 = top). */
static uint32_t s_rows[TW * TH];
static uint32_t s_px[TW * TH];

static GLuint compile(GLenum type, const char *src)
{
	GLuint s = glCreateShader(type);
	GLint ok = 0;
	char log[256];

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	if (!ok) {
		log[0] = 0;
		glGetShaderInfoLog(s, sizeof(log), NULL, log);
		pr_err("gl_test: shader %u compile failed: %s\n", (unsigned)type, log);
		return 0;
	}
	return s;
}

static GLuint link_program(const char *vs_src, const char *fs_src)
{
	GLuint vs = compile(GL_VERTEX_SHADER, vs_src);
	GLuint fs = compile(GL_FRAGMENT_SHADER, fs_src);
	GLuint prog;
	GLint ok = 0;

	if (!vs || !fs)
		return 0;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: link failed: %s\n", log);
		return 0;
	}
	return prog;
}

static void dump_map(const uint32_t *px, uint32_t w, uint32_t h)
{
	/* Dominant-channel map (AABBGGRR): R/G/B by strongest channel, 'W' when
	 * all three are bright, ' ' black. */
	for (uint32_t y = 0; y < h; y += 2) {
		char row[80];
		uint32_t n = 0;

		for (uint32_t x = 0; x < w && n < 78; x += 2) {
			uint32_t v = px[y * w + x];
			uint32_t rr = v & 0xff, gg = (v >> 8) & 0xff, bb = (v >> 16) & 0xff;

			if (rr < 8 && gg < 8 && bb < 8)
				row[n++] = ' ';
			else if (rr > 200 && gg > 200 && bb > 200)
				row[n++] = 'W';
			else if (rr >= gg && rr >= bb)
				row[n++] = 'R';
			else if (gg >= bb)
				row[n++] = 'G';
			else
				row[n++] = 'B';
		}
		row[n] = 0;
		pr_info("gl_test: px %s\n", row);
	}
}

/* Read the bound framebuffer (64x64) through glReadPixels and store it
 * top-down in s_px (glReadPixels rows are bottom-up in GL window space). */
static const uint32_t *read_topdown(void)
{
	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_rows);
	for (uint32_t y = 0; y < TH; y++)
		memcpy(&s_px[y * TW], &s_rows[(TH - 1 - y) * TW], TW * 4);
	return s_px;
}

static int chan_diff(uint32_t a, uint32_t b)
{
	int d = 0;

	for (int i = 0; i < 32; i += 8) {
		int x = (int)((a >> i) & 0xff) - (int)((b >> i) & 0xff);

		if (x < 0) x = -x;
		if (x > d) d = x;
	}
	return d;
}

/* One pixel check: top-down (x, y) of the last read_topdown() against `want`
 * with a per-channel tolerance. Prints the sample either way. */
static bool expect_px(const char *what, uint32_t x, uint32_t y, uint32_t want, int tol)
{
	uint32_t got = s_px[y * TW + x];
	bool ok = chan_diff(got, want) <= tol;

	pr_info("gl_test:   %-28s (%2u,%2u) = 0x%08x  want 0x%08x  %s  [check %d]\n",
		what, x, y, got, want, ok ? "ok" : "MISMATCH", gl_test_check_no + 1);
	return gl_test_check(ok, got, want);
}

static bool check_gl_error(const char *where)
{
	GLenum e = glGetError();

	if (e != GL_NO_ERROR) {
		pr_err("gl_test: GL error 0x%x after %s\n", (unsigned)e, where);
		return false;
	}
	return true;
}

/* ---- vertex layouts of the offline shaders ------------------------------ */

/* color_uv.vert: pos4 + color4 + uv2, 40 bytes */
struct cuv_vertex { float pos[4]; float color[4]; float uv[2]; };
/* passthrough.vert: pos4 + color4, 32 bytes */
struct pc_vertex { float pos[4]; float color[4]; };
/* uv.vert: pos4 + uv2, 24 bytes */
struct puv_vertex { float pos[4]; float uv[2]; };

static void cuv_set(struct cuv_vertex *v, float x, float y, float z,
		    float r, float g, float b)
{
	v->pos[0] = x; v->pos[1] = y; v->pos[2] = z; v->pos[3] = 1.0f;
	v->color[0] = r; v->color[1] = g; v->color[2] = b; v->color[3] = 1.0f;
	v->uv[0] = 1.0f; v->uv[1] = 1.0f;   /* color_uv.frag: o = color * (u, v, 1, 1) */
}

static void cuv_attribs(void)
{
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)16);
	glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)32);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
}

/* A full-screen quad (two triangles) with uv (0,0) at NDC (-1,-1). */
static void puv_quad(struct puv_vertex q[6])
{
	static const float c[6][4] = {
		{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
		{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
	};

	for (int i = 0; i < 6; i++) {
		q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1]; q[i].pos[2] = 0.0f; q[i].pos[3] = 1.0f;
		q[i].uv[0] = c[i][2]; q[i].uv[1] = c[i][3];
	}
}

static GLuint upload_vbo(const void *data, size_t bytes)
{
	GLuint vbo = 0;

	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, (GLsizeiptr)bytes, data, GL_STATIC_DRAW);
	return vbo;
}

/*
 * GL errors a case raised and never checked, by the case that raised them.
 *
 * reset_state() drains glGetError before every case. Whatever it finds was
 * left by the case before -- a real defect of that case or of the GL under
 * it, not noise -- and used to fail the NEXT case at its first
 * check_gl_error, then the one after, as a cascade that looked like a run of
 * broken cases (19-40 on the Zephyr i.MX8MP sweep, 2026-09-24). Each such
 * case gets its bit here, by case number, and the drain is printed naming
 * it. A sweep record has no room for it in its two bits per case, so a
 * sample that wants it on a board with no console reads these.
 */
uint32_t gl_test_leaked_error_cases[4];   /* bit c % 32 of word c / 32, c < 128 */
unsigned int gl_test_leaked_errors;        /* errors drained, all cases together */

static void drain_leaked_errors(int prev)
{
	for (int n = 0; n < 16; n++) {
		GLenum e = glGetError();

		if (e == GL_NO_ERROR)
			return;
		gl_test_leaked_errors++;
		if (prev >= 0 && prev < 128)
			gl_test_leaked_error_cases[prev / 32] |= 1u << (prev % 32);
		pr_err("gl_test: case %d left GL error 0x%x unchecked (drained before the next case)\n",
		       prev, (unsigned)e);
	}
}

/*
 * Every case starts from GL's defaults, whatever the case before it did or
 * failed to undo. A case that fails in the middle returns early and skips its
 * own unbinds and restores (there are dozens of early returns), and on a
 * board the whole sweep is one context: without this, one failure changed
 * the state every later case ran in. Only defaults are restored, so a case
 * that passed before still does.
 */
static void reset_state(int prev)
{
	GLint units = 0, attribs = 0;

	drain_leaked_errors(prev);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_CULL_FACE);
	glDisable(GL_SCISSOR_TEST);
	glDisable(GL_BLEND);
	glDepthMask(GL_TRUE);
	glDepthFunc(GL_LESS);
	glFrontFace(GL_CCW);
	glCullFace(GL_BACK);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	/* the write masks and the clear values a case may have changed: a case
	 * that fails in the middle does not restore its own */
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilMask(0xff);
	glDisable(GL_STENCIL_TEST);
	glClearDepthf(1.0f);
	glClearStencil(0);
	/* bindings */
	glUseProgram(0);
	glBindVertexArrayOES(0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glBindRenderbuffer(GL_RENDERBUFFER, 0);
	glGetIntegerv(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, &units);
	for (GLint u = 0; u < units && u < 32; u++) {
		glActiveTexture(GL_TEXTURE0 + (GLenum)u);
		glBindTexture(GL_TEXTURE_2D, 0);
		glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
	}
	glActiveTexture(GL_TEXTURE0);
	/* every generic attribute array of the default VAO disabled: "In the
	 * initial state, the boolean values are each false" (ES 3.0.6 2.9.3,
	 * the per-attribute enable flags). glBindVertexArrayOES(0) above makes
	 * the default VAO the one this edits. */
	glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &attribs);
	for (GLint a = 0; a < attribs && a < 32; a++)
		glDisableVertexAttribArray((GLuint)a);
	/* the rest of the fixed-function state, at GL's defaults */
	glStencilFuncSeparate(GL_FRONT_AND_BACK, GL_ALWAYS, 0, 0xff);
	glStencilOpSeparate(GL_FRONT_AND_BACK, GL_KEEP, GL_KEEP, GL_KEEP);
	glBlendFuncSeparate(GL_ONE, GL_ZERO, GL_ONE, GL_ZERO);
	glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_ADD);
	glBlendColor(0.0f, 0.0f, 0.0f, 0.0f);
	glDisable(GL_POLYGON_OFFSET_FILL);
	glPolygonOffset(0.0f, 0.0f);
	glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
	glDisable(GL_SAMPLE_COVERAGE);
	glEnable(GL_DITHER);
	glLineWidth(1.0f);
	glPixelStorei(GL_PACK_ALIGNMENT, 4);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	glDepthRangef(0.0f, 1.0f);
	/* the reset itself must not leave an error for the case to find */
	for (int n = 0; n < 16 && glGetError() != GL_NO_ERROR; n++)
		pr_err("gl_test: reset_state raised a GL error\n");
}

/* The slice-A triangle: v0 red apex, v1 green bottom-left, v2 blue bottom-right,
 * and the pixels the PVR_MULTIVARY rig proved for it (top-down coordinates). */
static const uint32_t A_V0 = 0xff090fe7, A_V1 = 0xff0fde13, A_V2 = 0xffe30913, A_CENTROID = 0xff595452;

static void slice_a_vertices(struct cuv_vertex v[3], int pretransform)
{
	if (pretransform) {
		/* screen space (pixels), z 0.5, w 1: the rig's proven triangle */
		cuv_set(&v[0], 32.0f, 8.0f, 0.5f, 1, 0, 0);
		cuv_set(&v[1], 8.0f, 56.0f, 0.5f, 0, 1, 0);
		cuv_set(&v[2], 56.0f, 56.0f, 0.5f, 0, 0, 1);
	} else {
		/* NDC clip space: apex at the top (y=+0.75 -> top rows of the image) */
		cuv_set(&v[0], 0.0f, 0.75f, 0.0f, 1, 0, 0);
		cuv_set(&v[1], -0.75f, -0.75f, 0.0f, 0, 1, 0);
		cuv_set(&v[2], 0.75f, -0.75f, 0.0f, 0, 0, 1);
	}
}

static bool check_slice_a_pixels(void)
{
	bool ok = true;

	ok &= expect_px("v0 red apex", 31, 12, A_V0, 3);
	ok &= expect_px("v1 green", 12, 52, A_V1, 3);
	ok &= expect_px("v2 blue", 52, 52, A_V2, 3);
	ok &= expect_px("centroid", 32, 40, A_CENTROID, 3);
	ok &= expect_px("background", 1, 1, 0xff000000, 0);
	return ok;
}

/* ---- case 0: slice A --------------------------------------------------- */

unsigned long pvr_regs(void);

static int case_triangle(int pretransform, int debug)
{
	struct cuv_vertex verts[3];
	GLuint prog, vbo;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	slice_a_vertices(verts, pretransform);
	vbo = upload_vbo(verts, sizeof(verts));
	if (debug) {
		struct gl_buffer *b = gl_lookup_buffer(vbo);
		const uint32_t *w = b ? (const uint32_t *)b->mem.cpu : NULL;

		for (int v = 0; w && v < 3; v++)
			pr_info("gl_test: vbo v%d: %08x %08x %08x %08x | %08x %08x %08x %08x | %08x %08x\n", v,
				w[v * 10 + 0], w[v * 10 + 1], w[v * 10 + 2], w[v * 10 + 3],
				w[v * 10 + 4], w[v * 10 + 5], w[v * 10 + 6], w[v * 10 + 7],
				w[v * 10 + 8], w[v * 10 + 9]);
	}
	cuv_attribs();
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw"))
		return -1;

	if (debug) {
		const struct osal_gpu_mem *m;
		const uint32_t *w;

		m = gl_draw_debug_usc(prog, 0);
		w = m ? (const uint32_t *)m->cpu : NULL;
		if (w)
			pr_info("gl_test: vs usc @0x%llx: %08x %08x %08x %08x %08x %08x %08x %08x %08x %08x\n",
				(unsigned long long)m->gpu, w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7], w[8], w[9]);
		m = gl_draw_debug_usc(prog, 1);
		w = m ? (const uint32_t *)m->cpu : NULL;
		if (w)
			pr_info("gl_test: fs usc @0x%llx: %08x %08x %08x %08x (52 dw) last %08x %08x\n",
				(unsigned long long)m->gpu, w[0], w[1], w[2], w[3], w[50], w[51]);
	}


	read_topdown();
	dump_map(s_px, TW, TH);
	return check_slice_a_pixels() ? 0 : -1;
}

/* ---- case 1: fragment uniform ------------------------------------------ */

static int case_uniform(void)
{
	struct pc_vertex verts[3];
	GLuint prog;
	GLint loc;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 3; i++) {
		static const float p[3][2] = { { 0.0f, 0.75f }, { -0.75f, -0.75f }, { 0.75f, -0.75f } };

		verts[i].pos[0] = p[i][0]; verts[i].pos[1] = p[i][1];
		verts[i].pos[2] = 0.0f; verts[i].pos[3] = 1.0f;
		verts[i].color[0] = verts[i].color[1] = verts[i].color[2] = verts[i].color[3] = 1.0f;
	}
	upload_vbo(verts, sizeof(verts));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	loc = glGetUniformLocation(prog, "u_color");
	if (loc < 0) {
		pr_err("gl_test: u_color location not found\n");
		return -1;
	}

	/* frame 1: orange */
	glUniform4f(loc, 1.0f, 0.5f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("orange centroid", 32, 40, 0xff0080ff, 2);
	ok &= expect_px("orange apex", 31, 12, 0xff0080ff, 2);
	ok &= expect_px("background", 1, 1, 0xff000000, 0);

	/* frame 2: the uniform changes -> cyan (the block is re-uploaded per draw) */
	glUniform4f(loc, 0.0f, 1.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("cyan centroid", 32, 40, 0xffffff00, 2);

	/* attribute reflection: names -> indices, and a relink with a_pos bound
	 * to index 3 (the app then feeds attribute 3) */
	{
		GLuint prog3 = glCreateProgram();
		GLuint vs3 = compile(GL_VERTEX_SHADER, gl_test_passthrough_vs_src);
		GLuint fs3 = compile(GL_FRAGMENT_SHADER, gl_test_uniform_fs_src);
		GLint ok3 = 0, la, lc, n = 0;
		char nm[32] = ""; GLenum ty = 0; GLint sz = 0;

		if (glGetAttribLocation(prog, "a_pos") != 0 || glGetAttribLocation(prog, "a_color") != 1 ||
		    glGetAttribLocation(prog, "nope") != -1) {
			pr_err("gl_test: glGetAttribLocation: %d %d %d\n", glGetAttribLocation(prog, "a_pos"),
			       glGetAttribLocation(prog, "a_color"), glGetAttribLocation(prog, "nope"));
			ok = false;
		}
		glGetProgramiv(prog, GL_ACTIVE_ATTRIBUTES, &n);
		glGetActiveAttrib(prog, 1, sizeof(nm), NULL, &sz, &ty, nm);
		pr_info("gl_test:   active attributes %d; [1] = %s size %d type 0x%x\n", n, nm, sz, (unsigned)ty);
		if (n != 2 || strcmp(nm, "a_color") != 0 || ty != GL_FLOAT_VEC4)
			ok = false;

		glAttachShader(prog3, vs3);
		glAttachShader(prog3, fs3);
		glBindAttribLocation(prog3, 3, "a_pos");
		glLinkProgram(prog3);
		glGetProgramiv(prog3, GL_LINK_STATUS, &ok3);
		la = glGetAttribLocation(prog3, "a_pos");
		lc = glGetAttribLocation(prog3, "a_color");
		pr_info("gl_test:   bound a_pos -> %d, a_color -> %d (link %d)\n", la, lc, ok3);
		if (!ok3 || la != 3 || lc != 1)
			ok = false;
		glUseProgram(prog3);
		glUniform4f(glGetUniformLocation(prog3, "u_color"), 1.0f, 0.0f, 1.0f, 1.0f);
		glDisableVertexAttribArray(0);
		glVertexAttribPointer(3, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
		glEnableVertexAttribArray(3);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		if (!check_gl_error("bound attrib draw"))
			return -1;
		read_topdown();
		ok &= expect_px("magenta via attribute 3", 32, 40, 0xffff00ff, 0);
		glDisableVertexAttribArray(3);
		glUseProgram(prog);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
		glEnableVertexAttribArray(0);
	}

	/* frames 3/4: a vertex-shader uniform (mvp.vert: gl_Position = u_mvp *
	 * a_pos, u_mvp a mat4 in the VS push-constant block) with the same FS */
	{
		GLuint prog2 = link_program(gl_test_mvp_vs_src, gl_test_uniform_fs_src);
		GLint lm, lc;
		/* column-major: scale by 0.5 */
		static const float scale_half[16] = {
			0.5f, 0, 0, 0,  0, 0.5f, 0, 0,  0, 0, 1, 0,  0, 0, 0, 1,
		};
		/* translate x by +0.5 */
		static const float shift_right[16] = {
			1, 0, 0, 0,  0, 1, 0, 0,  0, 0, 1, 0,  0.5f, 0, 0, 1,
		};

		if (!prog2)
			return -1;
		glUseProgram(prog2);
		lm = glGetUniformLocation(prog2, "u_mvp");
		lc = glGetUniformLocation(prog2, "u_color");
		if (lm < 0 || lc < 0) {
			pr_err("gl_test: u_mvp/u_color location %d/%d\n", lm, lc);
			return -1;
		}
		glDisableVertexAttribArray(1);   /* mvp.vert has only a_pos */
		glUniform4f(lc, 1.0f, 1.0f, 0.0f, 1.0f);
		glUniformMatrix4fv(lm, 1, GL_FALSE, scale_half);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		if (!check_gl_error("mvp draw 1"))
			return -1;
		read_topdown();
		dump_map(s_px, TW, TH);
		ok &= expect_px("half: inside small tri", 32, 40, 0xff00ffff, 0);
		ok &= expect_px("half: outside small tri", 32, 14, 0xff000000, 0);
		ok &= expect_px("half: outside small tri", 12, 52, 0xff000000, 0);

		glUniformMatrix4fv(lm, 1, GL_FALSE, shift_right);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		if (!check_gl_error("mvp draw 2"))
			return -1;
		read_topdown();
		ok &= expect_px("shift: moved right", 56, 40, 0xff00ffff, 0);
		ok &= expect_px("shift: left now empty", 16, 40, 0xff000000, 0);
		/* glGetActiveUniform reflection */
		{
			char nm[32] = ""; GLint sz = 0; GLenum ty = 0; GLint n = 0;

			glGetProgramiv(prog2, GL_ACTIVE_UNIFORMS, &n);
			glGetActiveUniform(prog2, (GLuint)lm, sizeof(nm), NULL, &sz, &ty, nm);
			pr_info("gl_test:   active uniforms %d; [%d] = %s size %d type 0x%x\n", n, lm, nm, sz, (unsigned)ty);
			if (n != 2 || strcmp(nm, "u_mvp") != 0 || ty != GL_FLOAT_MAT4 || sz != 1)
				ok = false;
		}
	}
	return ok ? 0 : -1;
}

/* ---- case 2: texture --------------------------------------------------- */

/* A size x size RGBA texture in four solid quadrants: row 0 (t = 0) side is
 * red | green, the t = 1 side is blue | white. NEAREST, clamp-to-edge. */
static GLuint make_quadrant_texture(uint32_t size)
{
	static uint8_t texels[64 * 64 * 4];
	GLuint tex = 0;

	for (uint32_t y = 0; y < size; y++) {
		for (uint32_t x = 0; x < size; x++) {
			uint8_t *p = &texels[(y * size + x) * 4];
			bool right = x >= size / 2, top = y >= size / 2;

			p[0] = (!top && !right) || (top && right) ? 255 : 0;
			p[1] = (!top && right) || (top && right) ? 255 : 0;
			p[2] = top ? 255 : 0;
			p[3] = 255;
		}
	}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, (GLsizei)size, (GLsizei)size, 0, GL_RGBA,
		     GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	{
		/* what actually landed in GPU memory (first two rows) */
		struct gl_texture *t = gl_lookup_texture(tex);
		const uint32_t *w = t ? (const uint32_t *)t->mem.cpu : NULL;

		if (w)
			pr_info("gl_test: %ux%u texture @0x%llx: %08x %08x %08x %08x | row1 %08x %08x\n",
				size, size, (unsigned long long)t->mem.gpu, w[0], w[1], w[2], w[3],
				w[size], w[size + 1]);
	}
	return tex;
}

static void puv_quad_setup(void)
{
	struct puv_vertex q[6];

	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
}

static int case_texture(void)
{
	static const uint32_t sizes[] = { 2, 4, 8, 64 };
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();

	for (unsigned i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		GLuint tex = make_quadrant_texture(sizes[i]);

		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("draw"))
			return -1;
		read_topdown();
		pr_info("gl_test: %ux%u texture on the full-screen quad:\n", sizes[i], sizes[i]);
		dump_map(s_px, TW, TH);
		/* t = 0 is the bottom of the screen: the red | green half lands in the
		 * lower half of the top-down image, blue | white in the upper half. */
		ok &= expect_px("top-left = blue", 16, 16, 0xffff0000, 0);
		ok &= expect_px("top-right = white", 48, 16, 0xffffffff, 0);
		ok &= expect_px("bottom-left = red", 16, 48, 0xff0000ff, 0);
		ok &= expect_px("bottom-right = green", 48, 48, 0xff00ff00, 0);
		glDeleteTextures(1, &tex);
	}

	/* An 8x8 texture with R = 36 x, G = 36 y, B = 128: the pixel values name
	 * the texel that was fetched, so a wrong iterator or stride shows as a
	 * wrong (x, y), not just a wrong colour. */
	{
		static uint8_t texels[8 * 8 * 4];
		GLuint tex = 0;

		for (uint32_t y = 0; y < 8; y++)
			for (uint32_t x = 0; x < 8; x++) {
				uint8_t *p = &texels[(y * 8 + x) * 4];

				p[0] = (uint8_t)(x * 36); p[1] = (uint8_t)(y * 36); p[2] = 128; p[3] = 255;
			}
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("gradient draw"))
			return -1;
		read_topdown();
		pr_info("gl_test: gradient texture (R = 36 x, G = 36 y, B = 128), 8x8 grid (top-down rows):\n");
		for (uint32_t y = 4; y < TH; y += 8) {
			pr_info("gl_test:  row %2u: %08x %08x %08x %08x %08x %08x %08x %08x\n", y,
				s_px[y * TW + 4], s_px[y * TW + 12], s_px[y * TW + 20], s_px[y * TW + 28],
				s_px[y * TW + 36], s_px[y * TW + 44], s_px[y * TW + 52], s_px[y * TW + 60]);
		}
		/* screen (4 + 8 i, 4 + 8 j) top-down -> texel (i, 7 - j) */
		ok &= expect_px("texel (0,7) at top-left", 4, 4, 0xff80fc00, 0);
		ok &= expect_px("texel (7,0) at bottom-right", 60, 60, 0xff8000fc, 0);
		ok &= expect_px("texel (2,5)", 20, 20, 0xff80b448, 0);
		ok &= expect_px("texel (5,2)", 44, 44, 0xff8048b4, 0);
		glDeleteTextures(1, &tex);
	}

	/* Mipmaps: a 4x4 texture whose levels are red (0), green (1), blue (2),
	 * NEAREST_MIPMAP_NEAREST. A magnified full-screen quad samples level 0;
	 * a 2x2-pixel quad (2 texels per pixel) level 1; a 1x1-pixel quad level 2.
	 * Then glGenerateMipmap on a half red / half blue base -> purple at level 2. */
	{
		static uint8_t red[4 * 4 * 4], green[2 * 2 * 4], blue[4], base[4 * 4 * 4];
		struct puv_vertex q[6];
		GLuint tex = 0;

		for (int i = 0; i < 16; i++) { red[i * 4] = 255; red[i * 4 + 3] = 255; }
		for (int i = 0; i < 4; i++) { green[i * 4 + 1] = 255; green[i * 4 + 3] = 255; }
		blue[2] = 255; blue[3] = 255;
		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, red);
		glTexImage2D(GL_TEXTURE_2D, 1, GL_RGBA, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, green);
		glTexImage2D(GL_TEXTURE_2D, 2, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, blue);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		if (!check_gl_error("mip upload"))
			return -1;

		glUseProgram(prog);
		puv_quad_setup();
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);        /* magnified: level 0 */
		glFinish();
		read_topdown();
		ok &= expect_px("mip: magnified -> level 0", 32, 32, 0xff0000ff, 0);

		/* a 2x2-pixel quad at pixels 30..31 (level 1) and a 1x1 at (40,40) (level 2) */
		for (int pass = 0; pass < 2; pass++) {
			float x0 = pass ? 40.0f : 30.0f, n = pass ? 1.0f : 2.0f;
			float l = x0 / TW * 2 - 1, r = (x0 + n) / TW * 2 - 1;
			float top = 1 - x0 / TH * 2, bot = 1 - (x0 + n) / TH * 2;
			const float c[6][4] = {
				{ l, bot, 0, 0 }, { r, bot, 1, 0 }, { r, top, 1, 1 },
				{ l, bot, 0, 0 }, { r, top, 1, 1 }, { l, top, 0, 1 },
			};

			for (int i = 0; i < 6; i++) {
				q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1]; q[i].pos[2] = 0; q[i].pos[3] = 1;
				q[i].uv[0] = c[i][2]; q[i].uv[1] = c[i][3];
			}
			upload_vbo(q, sizeof(q));
			glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
			glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
			read_topdown();
			if (pass == 0)
				ok &= expect_px("mip: 2x2 px quad -> level 1", 30, 30, 0xff00ff00, 0);
			else
				ok &= expect_px("mip: 1x1 px quad -> level 2", 40, 40, 0xffff0000, 0);
		}

		/* glGenerateMipmap: left half red, right half blue -> level 2 purple */
		for (int y = 0; y < 4; y++)
			for (int x = 0; x < 4; x++) {
				base[(y * 4 + x) * 4 + 0] = x < 2 ? 255 : 0;
				base[(y * 4 + x) * 4 + 2] = x < 2 ? 0 : 255;
				base[(y * 4 + x) * 4 + 3] = 255;
			}
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, base);
		glGenerateMipmap(GL_TEXTURE_2D);
		if (!check_gl_error("glGenerateMipmap"))
			return -1;
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);        /* still the 1x1 quad */
		glFinish();
		read_topdown();
		ok &= expect_px("generated level 2 = average", 40, 40, 0xff800080, 1);
		/* trilinear at an exact level lands on that level */
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		ok &= expect_px("trilinear at level 2", 40, 40, 0xff800080, 1);
		glDeleteTextures(1, &tex);
		puv_quad_setup();
	}

	/* Two texture units: texture2.frag = 0.5 * unit-0 texture + 0.25 * unit-1
	 * texture. Unit 0 = the 4x4 quadrant texture, unit 1 = a 1x1 yellow one;
	 * then the sampler uniforms are swapped. */
	{
		static const uint8_t yellow[4] = { 255, 255, 0, 255 };
		GLuint prog2 = link_program(gl_test_uv_vs_src, gl_test_texture2_fs_src);
		GLuint quad, yel = 0;
		GLint l0, l1;

		if (!prog2)
			return -1;
		glUseProgram(prog2);
		glActiveTexture(GL_TEXTURE0);
		quad = make_quadrant_texture(4);
		glActiveTexture(GL_TEXTURE1);
		glGenTextures(1, &yel);
		glBindTexture(GL_TEXTURE_2D, yel);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, yellow);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glActiveTexture(GL_TEXTURE0);
		l0 = glGetUniformLocation(prog2, "u_tex0");
		l1 = glGetUniformLocation(prog2, "u_tex1");
		if (l0 < 0 || l1 < 0) {
			pr_err("gl_test: sampler locations %d %d\n", l0, l1);
			return -1;
		}
		glUniform1i(l0, 0);
		glUniform1i(l1, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("two units draw 1"))
			return -1;
		read_topdown();
		ok &= expect_px("2 units: red*0.5 + yellow*0.25", 16, 48, 0xff0040bf, 2);
		ok &= expect_px("2 units: blue*0.5 + yellow*0.25", 16, 16, 0xff804040, 2);

		glUniform1i(l0, 1);
		glUniform1i(l1, 0);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("two units draw 2"))
			return -1;
		read_topdown();
		ok &= expect_px("swapped: yellow*0.5 + red*0.25", 16, 48, 0xff0080bf, 2);
		ok &= expect_px("swapped: yellow*0.5 + blue*0.25", 16, 16, 0xff408080, 2);
		{
			GLint u = -1;

			glGetUniformiv(prog2, l0, &u);
			if (u != 1) {
				pr_err("gl_test: glGetUniformiv(u_tex0) = %d, want 1\n", u);
				ok = false;
			}
		}
		glDeleteTextures(1, &quad);
		glDeleteTextures(1, &yel);
	}
	return ok ? 0 : -1;
}

/* ---- case 3: depth test ------------------------------------------------ */

/* A per-iteration label for expect_px(). Must fit the longest one in this
 * file, "layer %d (rgb texture, a texelFetch)" at 37 bytes with a one-digit
 * layer -- it was 32 and silently truncated, which gcc caught as
 * "__builtin___snprintf_chk output 36 bytes into a destination of size 32".
 * One constant rather than eight literals, so the next label added to any of
 * them is measured against the same bound. */
#define GL_TEST_LABEL 64

static int case_depth(void)
{
	/* A: big red triangle at z 0; B: medium green at z 0.5 (behind A);
	 * C: small blue at z -0.5 (in front of A). All drawn in the order A, B, C. */
	struct cuv_vertex v[12];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	/* D: A's triangle again, green: coplanar with A for the polygon offset */
	cuv_set(&v[9], -0.9f, -0.9f, 0.0f, 0, 1, 0);
	cuv_set(&v[10], 0.9f, -0.9f, 0.0f, 0, 1, 0);
	cuv_set(&v[11], 0.0f, 0.9f, 0.0f, 0, 1, 0);
	cuv_set(&v[0], -0.9f, -0.9f, 0.0f, 1, 0, 0);
	cuv_set(&v[1], 0.9f, -0.9f, 0.0f, 1, 0, 0);
	cuv_set(&v[2], 0.0f, 0.9f, 0.0f, 1, 0, 0);
	cuv_set(&v[3], -0.5f, -0.5f, 0.5f, 0, 1, 0);
	cuv_set(&v[4], 0.5f, -0.5f, 0.5f, 0, 1, 0);
	cuv_set(&v[5], 0.0f, 0.5f, 0.5f, 0, 1, 0);
	cuv_set(&v[6], -0.25f, -0.25f, -0.5f, 0, 0, 1);
	cuv_set(&v[7], 0.25f, -0.25f, -0.5f, 0, 0, 1);
	cuv_set(&v[8], 0.0f, 0.25f, -0.5f, 0, 0, 1);
	upload_vbo(v, sizeof(v));
	cuv_attribs();

	/* frame 1: depth test on -> B is hidden behind A, C shows in front */
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 9);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("depth on: C in front", 32, 32, 0xffff0000, 0);
	ok &= expect_px("depth on: B hidden", 32, 44, 0xff0000ff, 0);
	ok &= expect_px("depth on: A only", 32, 54, 0xff0000ff, 0);

	/* frame 2: depth test off -> draw order wins, B shows over A */
	glDisable(GL_DEPTH_TEST);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 9);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("depth off: C last", 32, 32, 0xffff0000, 0);
	ok &= expect_px("depth off: B over A", 32, 44, 0xff00ff00, 0);
	ok &= expect_px("depth off: A only", 32, 54, 0xff0000ff, 0);

	/* frame 3: depth test on, GL_GREATER -> B (farther) wins over A, C loses */
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_GREATER);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	/* the cleared depth is 1.0 (the farthest), so GREATER needs a nearer
	 * clear: glClearDepthf(0) */
	glClearDepthf(0.0f);
	glClear(GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 9);
	glFinish();
	glClearDepthf(1.0f);
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	ok &= expect_px("greater: B over A, C", 32, 32, 0xff00ff00, 0);
	ok &= expect_px("greater: B over A", 32, 44, 0xff00ff00, 0);
	ok &= expect_px("greater: A only", 32, 54, 0xff0000ff, 0);

	/* frames 4/5: D is coplanar with A, so LESS hides it; a negative
	 * polygon offset pulls it in front, a positive one keeps it behind */
	glDepthFunc(GL_LESS);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glEnable(GL_POLYGON_OFFSET_FILL);
	glPolygonOffset(0.0f, -4.0f);
	glDrawArrays(GL_TRIANGLES, 9, 3);
	glFinish();
	if (!check_gl_error("draw 4"))
		return -1;
	read_topdown();
	ok &= expect_px("offset -4: D in front of A", 32, 54, 0xff00ff00, 0);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glPolygonOffset(0.0f, 0.0f);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glPolygonOffset(0.0f, 4.0f);
	glDrawArrays(GL_TRIANGLES, 9, 3);
	glFinish();
	if (!check_gl_error("draw 5"))
		return -1;
	read_topdown();
	ok &= expect_px("offset +4: A stays", 32, 54, 0xff0000ff, 0);
	glDisable(GL_POLYGON_OFFSET_FILL);
	glDisable(GL_DEPTH_TEST);
	return ok ? 0 : -1;
}

/* ---- case 53: nothing but a depth clear, for the value probe ------------ */

/* What to clear to, from Kconfig where there is one (the host build has not).
 * A second value is what separates "the clear was ignored" from "the surface
 * is not uniform", since those share a reading at 1.0. */
#ifdef CONFIG_GC_GL_TEST_ZPROBE_MILLI
#define GL_TEST_ZPROBE_DEPTH ((float)CONFIG_GC_GL_TEST_ZPROBE_MILLI / 1000.0f)
#else
#define GL_TEST_ZPROBE_DEPTH 0.5f
#endif

static int case_zprobe(void)
{
	/* Two ways a depth clear reaches the GPU, and they are different code
	 * paths in the GL core:
	 *
	 *   A  the clear has no draw behind it, and glFinish has to carry it
	 *      out -- gl_flush_pending_clear builds a draw of its own
	 *   B  a draw follows the clear, and takes the clear with it
	 *
	 * Both are exercised here because covering only one hid a real bug.
	 * The depth-value probe was written to isolate path A, and isolating
	 * it moved the clear onto a path whose depth range was never set, so
	 * every clear stored 0 there while path B was correct. A probe that
	 * changes the path under test measures the probe.
	 *
	 * Each half clears depth to 0.5, then draws a near quad that should
	 * pass GL_LESS and a far one that should not. The colour at the centre
	 * says whether the clear value survived.
	 *
	 * The case ends with a clear and no draw, so the surface is left
	 * holding one value for GC_GL_TEST_DEPTH_VALUE to read back. That
	 * reading therefore describes path A; there is no reading of path B,
	 * only these checks.
	 */
	struct cuv_vertex v[12];
	GLuint prog;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	/* 0..5 near quad, window z 0.4, blue; 6..11 far quad, 0.9, green */
	for (int i = 0; i < 6; i++) {
		cuv_set(&v[i], c[i][0], c[i][1], -0.2f, 0, 0, 1);
		cuv_set(&v[6 + i], c[i][0], c[i][1], 0.8f, 0, 1, 0);
	}
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);

	/* A: the clear is carried out on its own, then drawn over */
	glClearDepthf(0.5f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("zprobe clear alone"))
		return -1;
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDrawArrays(GL_TRIANGLES, 6, 6);
	glFinish();
	if (!check_gl_error("zprobe draw after a lone clear"))
		return -1;
	read_topdown();
	ok &= expect_px("clear alone: near quad passes", 32, 32, 0xffff0000, 0);
	ok &= expect_px("clear alone: far quad fails", 4, 4, 0xffff0000, 0);

	/* B: the same, with the draws taking the clear with them */
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDrawArrays(GL_TRIANGLES, 6, 6);
	glFinish();
	if (!check_gl_error("zprobe draw carrying the clear"))
		return -1;
	read_topdown();
	ok &= expect_px("clear with a draw: near quad passes", 32, 32, 0xffff0000, 0);
	ok &= expect_px("clear with a draw: far quad fails", 4, 4, 0xffff0000, 0);

	/* leave the surface uniform for the depth-value probe (path A) */
	glClearDepthf(GL_TEST_ZPROBE_DEPTH);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glFinish();
	glClearDepthf(1.0f);
	glDisable(GL_DEPTH_TEST);
	return ok && check_gl_error("zprobe end") ? 0 : -1;
}

/* ---- case 4: face culling ---------------------------------------------- */

static int case_cull(void)
{
	/* one yellow triangle wound counter-clockwise in GL window space */
	struct cuv_vertex v[3];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	cuv_set(&v[0], -0.75f, -0.75f, 0.0f, 1, 1, 0);
	cuv_set(&v[1], 0.75f, -0.75f, 0.0f, 1, 1, 0);
	cuv_set(&v[2], 0.0f, 0.75f, 0.0f, 1, 1, 0);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glEnable(GL_CULL_FACE);

	/* frame 1: front = CCW, cull back -> visible */
	glFrontFace(GL_CCW);
	glCullFace(GL_BACK);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("ccw front, cull back", 32, 40, 0xff00ffff, 0);

	/* frame 2: cull front -> culled */
	glCullFace(GL_FRONT);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("ccw front, cull front", 32, 40, 0xff000000, 0);

	/* frame 3: front = CW, cull back -> the CCW triangle is a back face: culled */
	glFrontFace(GL_CW);
	glCullFace(GL_BACK);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	ok &= expect_px("cw front, cull back", 32, 40, 0xff000000, 0);

	/* frame 4: front = CW, cull front -> visible */
	glCullFace(GL_FRONT);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("draw 4"))
		return -1;
	read_topdown();
	ok &= expect_px("cw front, cull front", 32, 40, 0xff00ffff, 0);

	/* frames 5/6: the same triangle into a framebuffer texture (no y flip):
	 * the GL winding rule must still hold there */
	{
		GLuint tex = 0, fbo = 0;

		glGenTextures(1, &tex);
		glBindTexture(GL_TEXTURE_2D, tex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glGenFramebuffers(1, &fbo);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
		glFrontFace(GL_CCW);
		glCullFace(GL_BACK);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		read_topdown();
		ok &= expect_px("fbo: ccw front, cull back", 32, 32, 0xff00ffff, 0);
		glCullFace(GL_FRONT);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		read_topdown();
		ok &= expect_px("fbo: ccw front, cull front", 32, 32, 0xff000000, 0);
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteFramebuffers(1, &fbo);
		glDeleteTextures(1, &tex);
		if (!check_gl_error("fbo cull"))
			ok = false;
	}
	glDisable(GL_CULL_FACE);
	return ok ? 0 : -1;
}

/* ---- case 5: scissor --------------------------------------------------- */

static int case_scissor(void)
{
	struct cuv_vertex q[6];
	GLuint prog;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++)
		cuv_set(&q[i], c[i][0], c[i][1], 0.0f, 1, 1, 0);
	upload_vbo(q, sizeof(q));
	cuv_attribs();

	/* The background the "outside the box" checks below read. It has to be
	 * established here: everything after this point is confined to the
	 * scissor box, so without it the case is asking what some earlier case
	 * happened to leave behind -- which is nothing at all when the case runs
	 * first, or in a process of its own.
	 *
	 * It used to pass without this. Before glClear honoured the scissor
	 * (99092c7) the scissored clear below covered the whole target, so it
	 * painted this background as a side effect of not working. The finish is
	 * deliberate: it puts the clear out before the scissor is enabled, so
	 * this is one clear and the next is another. */
	glDisable(GL_SCISSOR_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();

	/* the yellow full-screen quad clipped to the bottom-left 32x32 (GL window
	 * coordinates: y up) -> the lower-left quadrant of the top-down image */
	glEnable(GL_SCISSOR_TEST);
	glScissor(0, 0, 32, 32);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	if (!check_gl_error("draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("inside (bottom-left)", 16, 48, 0xff00ffff, 0);
	ok &= expect_px("inside edge", 31, 32, 0xff00ffff, 0);
	ok &= expect_px("outside right", 32, 32, 0xff000000, 0);
	ok &= expect_px("outside above", 16, 31, 0xff000000, 0);
	ok &= expect_px("outside (top-right)", 48, 16, 0xff000000, 0);
	return ok ? 0 : -1;
}

/* ---- case 6: framebuffer object ---------------------------------------- */

static int case_fbo(void)
{
	struct cuv_vertex verts[3];
	GLuint tri_prog, tex_prog, tex = 0, fbo = 0;
	GLenum status;
	bool ok = true;
	static uint32_t fbo_px[TW * TH];

	tri_prog = link_program(gl_test_vs_src, gl_test_fs_src);
	tex_prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!tri_prog || !tex_prog)
		return -1;

	/* the render target: an empty 64x64 RGBA texture */
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status != GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT) {
		pr_err("gl_test: empty FBO status 0x%x\n", (unsigned)status);
		ok = false;
	}
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: FBO status 0x%x\n", (unsigned)status);
		return -1;
	}

	/* pass 1: the slice-A triangle into the texture */
	glUseProgram(tri_prog);
	slice_a_vertices(verts, 0);
	upload_vbo(verts, sizeof(verts));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("pass 1"))
		return -1;
	read_topdown();
	pr_info("gl_test: FBO texture contents (via glReadPixels on the FBO):\n");
	dump_map(s_px, TW, TH);
	ok &= check_slice_a_pixels();
	memcpy(fbo_px, s_px, sizeof(fbo_px));

	/* pass 2: sample that texture on a full-screen quad into the window */
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glUseProgram(tex_prog);
	glBindTexture(GL_TEXTURE_2D, tex);
	glDisableVertexAttribArray(2);
	puv_quad_setup();
	glClearColor(0.25f, 0.25f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("pass 2"))
		return -1;
	read_topdown();
	pr_info("gl_test: window contents (the texture sampled 1:1):\n");
	dump_map(s_px, TW, TH);
	ok &= check_slice_a_pixels();
	{
		uint32_t bad = 0, first = 0;

		for (uint32_t i = 0; i < TW * TH; i++) {
			if (s_px[i] != fbo_px[i]) {
				if (!bad)
					first = i;
				bad++;
			}
		}
		pr_info("gl_test:   %-28s %u pixel(s) differ%s\n", "sampled == rendered",
			bad, bad ? "  MISMATCH" : "");
		if (bad) {
			pr_info("gl_test:   first at (%u,%u): 0x%08x vs 0x%08x\n", first % TW, first / TW,
				s_px[first], fbo_px[first]);
			ok = false;
		}
	}

	/* the attachment query + delete path */
	{
		GLint type = 0, name = 0;

		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
						      GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE, &type);
		glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
						      GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME, &name);
		if (type != GL_TEXTURE || name != (GLint)tex) {
			pr_err("gl_test: attachment query type 0x%x name %d\n", (unsigned)type, name);
			ok = false;
		}
		glDeleteTextures(1, &tex);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT) {
			pr_err("gl_test: FBO still complete after its texture was deleted\n");
			ok = false;
		}
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteFramebuffers(1, &fbo);
	}
	return ok && check_gl_error("fbo teardown") ? 0 : -1;
}

/* ---- case 8: blending -------------------------------------------------- */

static int case_blend(void)
{
	/* an opaque red full-screen quad, then a half-transparent green
	 * triangle over it with different blend functions */
	struct cuv_vertex v[9];
	GLuint prog;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++)
		cuv_set(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
	cuv_set(&v[6], -0.75f, -0.75f, 0.0f, 0, 1, 0);
	cuv_set(&v[7], 0.75f, -0.75f, 0.0f, 0, 1, 0);
	cuv_set(&v[8], 0.0f, 0.75f, 0.0f, 0, 1, 0);
	for (int i = 6; i < 9; i++)
		v[i].color[3] = 0.5f;
	upload_vbo(v, sizeof(v));
	cuv_attribs();

	/* frame 1: SRC_ALPHA / ONE_MINUS_SRC_ALPHA -> 0.5 red + 0.5 green,
	 * alpha 0.5 * 0.5 + 1 * 0.5 = 0.75 */
	glClear(GL_COLOR_BUFFER_BIT);
	glDisable(GL_BLEND);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_BLEND);
	glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("over: inside triangle", 32, 40, 0xbf008080, 2);
	ok &= expect_px("over: outside (red quad)", 4, 4, 0xff0000ff, 0);

	/* frame 2: ONE / ONE (additive) -> red + green */
	glDisable(GL_BLEND);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_BLEND);
	glBlendFunc(GL_ONE, GL_ONE);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("additive: inside triangle", 32, 40, 0xff00ffff, 2);

	/* frame 3: ONE / ONE_MINUS_SRC_ALPHA (premultiplied) -> green + 0.5 red */
	glDisable(GL_BLEND);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_BLEND);
	glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glFinish();
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	ok &= expect_px("premultiplied: inside", 32, 40, 0xff00ff80, 2);

	/* frame 4: blending on with ONE / ZERO is a plain replace (alpha 0.5 lands) */
	glDisable(GL_BLEND);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_BLEND);
	glBlendFunc(GL_ONE, GL_ZERO);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glFinish();
	if (!check_gl_error("draw 4"))
		return -1;
	read_topdown();
	ok &= expect_px("replace: inside", 32, 40, 0x8000ff00, 0);

	/* frame 5: CONSTANT_COLOR / ONE_MINUS_CONSTANT_COLOR with glBlendColor
	 * (0.25, 0.5, 0.75, 1): r = 0.75 red, g = 0.5 green, a = 0.5 * 1 */
	glDisable(GL_BLEND);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_BLEND);
	glBlendColor(0.25f, 0.5f, 0.75f, 1.0f);
	glBlendFunc(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glFinish();
	if (!check_gl_error("draw 5"))
		return -1;
	read_topdown();
	ok &= expect_px("constant colour: inside", 32, 40, 0x800080bf, 2);

	/* frame 6: colour mask G+A only, no blending: the green triangle
	 * writes g and a over the red quad -> (1, 1, 0, 0.5) */
	glDisable(GL_BLEND);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glColorMask(GL_FALSE, GL_TRUE, GL_FALSE, GL_TRUE);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glFinish();
	if (!check_gl_error("draw 6"))
		return -1;
	read_topdown();
	ok &= expect_px("mask ga: inside", 32, 40, 0x8000ffff, 0);
	ok &= expect_px("mask ga: outside untouched", 4, 4, 0xff0000ff, 0);

	/* frame 7: SRC_ALPHA / ONE_MINUS_SRC_ALPHA with alpha masked off:
	 * rgb blend as frame 1, alpha stays the quad's 1 */
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_BLEND);
	glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glFinish();
	if (!check_gl_error("draw 7"))
		return -1;
	read_topdown();
	ok &= expect_px("over, alpha masked: inside", 32, 40, 0xff008080, 2);
	glDisable(GL_BLEND);
	return ok ? 0 : -1;
}

/* ---- case 9: stencil ---------------------------------------------------- */

static int case_stencil(void)
{
	/* vertices 0..5: full-screen yellow quad; 6..8: a mask triangle */
	struct cuv_vertex v[9];
	GLuint prog;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++)
		cuv_set(&v[i], c[i][0], c[i][1], 0.0f, 1, 1, 0);
	cuv_set(&v[6], -0.75f, -0.75f, 0.0f, 1, 0, 0);
	cuv_set(&v[7], 0.75f, -0.75f, 0.0f, 1, 0, 0);
	cuv_set(&v[8], 0.0f, 0.75f, 0.0f, 1, 0, 0);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glEnable(GL_STENCIL_TEST);

	/* frame 1: mask the triangle into the stencil (no colour), then the
	 * quad only where stencil == 1 */
	glClearStencil(0);
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFunc(GL_ALWAYS, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("equal 1: inside mask", 32, 40, 0xff00ffff, 0);
	ok &= expect_px("equal 1: outside mask", 4, 4, 0xff000000, 0);
	ok &= expect_px("mask pass wrote no colour", 32, 50, 0xff00ffff, 0);

	/* frame 2: NOTEQUAL 1 -> the quad everywhere but the mask */
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFunc(GL_ALWAYS, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilFunc(GL_NOTEQUAL, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("notequal 1: inside mask", 32, 40, 0xff000000, 0);
	ok &= expect_px("notequal 1: outside mask", 4, 4, 0xff00ffff, 0);

	/* frame 3: INCR twice (stencil 2 inside the triangle), quad where == 2 */
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFunc(GL_ALWAYS, 0, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilFunc(GL_EQUAL, 2, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	ok &= expect_px("incr x2: inside == 2", 32, 40, 0xff00ffff, 0);
	ok &= expect_px("incr x2: outside == 0", 4, 4, 0xff000000, 0);

	/* frame 4: glClearStencil(1) reaches the background stencil */
	glClearStencil(1);
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	glClearStencil(0);
	if (!check_gl_error("draw 4"))
		return -1;
	read_topdown();
	ok &= expect_px("cleared to 1: quad everywhere", 4, 4, 0xff00ffff, 0);

	/* frame 5: the write mask keeps bit 0 out -> REPLACE 1 with mask 0xfe
	 * leaves 0, so EQUAL 1 draws nothing */
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glStencilMask(0xfe);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFunc(GL_ALWAYS, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
	glDrawArrays(GL_TRIANGLES, 6, 3);
	glStencilMask(0xff);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 5"))
		return -1;
	read_topdown();
	ok &= expect_px("write mask 0xfe: nothing", 32, 40, 0xff000000, 0);
	glDisable(GL_STENCIL_TEST);
	return ok ? 0 : -1;
}

/* ---- case 12: two-sided stencil (glStencil*Separate) ------------------- */

/* Marks the left triangle (counter-clockwise in GL window space) and the
 * right one (clockwise) into the stencil with the current separate state,
 * then reveals stencil == 1 in red and stencil == 2 in blue. */
static void stencil2_frame(void)
{
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFuncSeparate(GL_FRONT, GL_ALWAYS, 1, 0xff);
	glStencilFuncSeparate(GL_BACK, GL_ALWAYS, 2, 0xff);
	glStencilOpSeparate(GL_FRONT_AND_BACK, GL_KEEP, GL_KEEP, GL_REPLACE);
	glDrawArrays(GL_TRIANGLES, 6, 6);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glDrawArrays(GL_TRIANGLES, 0, 6);      /* red quad where == 1 */
	glStencilFunc(GL_EQUAL, 2, 0xff);
	glDrawArrays(GL_TRIANGLES, 12, 6);     /* blue quad where == 2 */
	glFinish();
}

static int case_stencil2(void)
{
	/* 0..5 red quad, 6..8 left CCW triangle, 9..11 right CW triangle,
	 * 12..17 blue quad */
	struct cuv_vertex v[18];
	GLuint prog, fbo, tex, sten;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		cuv_set(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
		cuv_set(&v[12 + i], c[i][0], c[i][1], 0.0f, 0, 0, 1);
	}
	cuv_set(&v[6], -0.9f, -0.8f, 0.0f, 1, 1, 1);
	cuv_set(&v[7], -0.1f, -0.8f, 0.0f, 1, 1, 1);
	cuv_set(&v[8], -0.5f, 0.8f, 0.0f, 1, 1, 1);
	cuv_set(&v[9], 0.1f, -0.8f, 0.0f, 1, 1, 1);
	cuv_set(&v[10], 0.5f, 0.8f, 0.0f, 1, 1, 1);
	cuv_set(&v[11], 0.9f, -0.8f, 0.0f, 1, 1, 1);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glClearStencil(0);
	glEnable(GL_STENCIL_TEST);

	/* frame 1: front = CCW (default): left red, right blue */
	glFrontFace(GL_CCW);
	stencil2_frame();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("ccw front: left (front) == 1 red", 16, 40, 0xff0000ff, 0);
	ok &= expect_px("ccw front: right (back) == 2 blue", 48, 40, 0xffff0000, 0);
	ok &= expect_px("ccw front: outside untouched", 32, 4, 0xff000000, 0);

	/* frame 2: front = CW: the faces swap */
	glFrontFace(GL_CW);
	stencil2_frame();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("cw front: left (back) == 2 blue", 16, 40, 0xffff0000, 0);
	ok &= expect_px("cw front: right (front) == 1 red", 48, 40, 0xff0000ff, 0);

	/* frame 3: the same into an FBO (no y flip: the winding the TA sees
	 * is inverted, the backend must compensate as it does for culling).
	 *
	 * The framebuffer needs a stencil attachment of its own: a framebuffer
	 * object has only the buffers it was given, and with no stencil buffer
	 * the stencil test always passes and nothing is written (ES 2.0 4.1.4).
	 * Until 2026-09-22 the target's own depth/stencil buffer stood in for
	 * one that GL says is not there, and this frame passed on that. */
	glFrontFace(GL_CCW);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenRenderbuffers(1, &sten);
	glBindRenderbuffer(GL_RENDERBUFFER, sten);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, TW, TH);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, sten);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: stencil2 FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, TW, TH);
	stencil2_frame();
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	/* the FBO is read back bottom-up as well; the triangles are symmetric
	 * about the horizontal centre line so the rows do not matter */
	ok &= expect_px("fbo ccw front: left == 1 red", 16, 40, 0xff0000ff, 0);
	ok &= expect_px("fbo ccw front: right == 2 blue", 48, 40, 0xffff0000, 0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteRenderbuffers(1, &sten);
	glDeleteTextures(1, &tex);

	/* frame 4: single-faced draw still works with separate state armed
	 * (lines use the front state) */
	glStencilFunc(GL_ALWAYS, 0, 0xff);
	glDisable(GL_STENCIL_TEST);
	return ok ? 0 : -1;
}

/* NDC of the centre of top-down pixel (x, y) */
static float ndc_x(uint32_t x) { return ((float)x + 0.5f) / (float)TW * 2.0f - 1.0f; }
static float ndc_y(uint32_t y) { return 1.0f - ((float)y + 0.5f) / (float)TH * 2.0f; }

/* ---- case 13: depth/stencil survive across kicks (ISP Z load/store) ------ */

static int case_zls(void)
{
	/* 0..2 red near triangle (window z 0.2), 3..8 green far quad (0.9),
	 * 9..14 blue mid quad (0.4), 15.. 140 yellow small far triangles (0.8)
	 * on the case-11 grid: more than one kick's worth of draws */
	static struct cuv_vertex v[15 + 3 * 140];
	GLuint prog;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	cuv_set(&v[0], -0.9f, -0.9f, -0.6f, 1, 0, 0);
	cuv_set(&v[1], 0.9f, -0.9f, -0.6f, 1, 0, 0);
	cuv_set(&v[2], 0.0f, 0.9f, -0.6f, 1, 0, 0);
	for (int i = 0; i < 6; i++) {
		cuv_set(&v[3 + i], c[i][0], c[i][1], 0.8f, 0, 1, 0);
		cuv_set(&v[9 + i], c[i][0], c[i][1], -0.2f, 0, 0, 1);
	}
	for (int i = 0; i < 140; i++) {
		float cx = 4.0f + (float)(i % 20) * 3.0f, cy = 4.0f + (float)(i / 20) * 4.0f;

		cuv_set(&v[15 + i * 3 + 0], ndc_x((uint32_t)cx) - 1.0f / TW, ndc_y((uint32_t)cy) - 1.0f / TH, 0.6f, 1, 1, 0);
		cuv_set(&v[15 + i * 3 + 1], ndc_x((uint32_t)cx) + 3.0f / TW, ndc_y((uint32_t)cy) - 1.0f / TH, 0.6f, 1, 1, 0);
		cuv_set(&v[15 + i * 3 + 2], ndc_x((uint32_t)cx) - 1.0f / TW, ndc_y((uint32_t)cy) + 3.0f / TH, 0.6f, 1, 1, 0);
	}
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);

	/* frame 1: glClearDepthf(0.5) is honoured in the stored format: the
	 * mid quad (0.4) passes, the far quad (0.9) fails */
	glClearDepthf(0.5f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 9, 6);
	glDrawArrays(GL_TRIANGLES, 3, 6);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	ok &= expect_px("clear 0.5: mid quad passes", 32, 32, 0xffff0000, 0);
	ok &= expect_px("clear 0.5: far quad fails", 4, 4, 0xffff0000, 0);
	glClearDepthf(1.0f);

	/* frame 2: the near triangle, 140 small far triangles (the frame splits
	 * into two kicks), then the far quad: the triangle and the small ones
	 * from the first kick must still win the depth test */
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	for (int i = 0; i < 140; i++)
		glDrawArrays(GL_TRIANGLES, 15 + i * 3, 3);
	glDrawArrays(GL_TRIANGLES, 3, 6);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("split: near triangle kept", 32, 32, 0xff0000ff, 0);
	ok &= expect_px("split: kick-1 small tri kept", 4, 4, 0xff00ffff, 0);
	ok &= expect_px("split: kick-2 small tri", 4 + 3 * 19, 4 + 4 * 6, 0xff00ffff, 0);
	ok &= expect_px("split: far quad elsewhere", 60, 40, 0xff00ff00, 0);

	/* frame 3: stencil written in one kick is still there in the next
	 * (no clear between: colour, depth and stencil are all reloaded) */
	glDisable(GL_DEPTH_TEST);
	glEnable(GL_STENCIL_TEST);
	glClearStencil(0);
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFunc(GL_ALWAYS, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDrawArrays(GL_TRIANGLES, 3, 6);
	glFinish();
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	ok &= expect_px("stencil kept: inside", 32, 32, 0xff00ff00, 0);
	ok &= expect_px("stencil kept: outside", 4, 4, 0xff000000, 0);
	glDisable(GL_STENCIL_TEST);

	/* frame 4: a depth clear between draws starts a new kick that keeps
	 * the colour but not the depth */
	glEnable(GL_DEPTH_TEST);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glClear(GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 3, 6);
	glFinish();
	if (!check_gl_error("draw 4"))
		return -1;
	read_topdown();
	ok &= expect_px("depth cleared mid-frame: far quad wins", 32, 32, 0xff00ff00, 0);

	/* frame 5: glDepthRangef(0, 0.3) maps the far quad (NDC 0.8) to 0.27:
	 * it passes LESS against the 0.5 clear, then the range is restored */
	glClearDepthf(0.5f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDepthRangef(0.0f, 0.3f);
	glDrawArrays(GL_TRIANGLES, 3, 6);
	glDepthRangef(0.0f, 1.0f);
	glDrawArrays(GL_TRIANGLES, 9, 6);   /* mid quad at 0.4: fails against 0.27 */
	glFinish();
	glClearDepthf(1.0f);
	if (!check_gl_error("draw 5"))
		return -1;
	read_topdown();
	ok &= expect_px("depth range 0..0.3: far quad wins", 32, 32, 0xff00ff00, 0);
	glDisable(GL_DEPTH_TEST);
	return ok ? 0 : -1;
}

/* ---- case 14: the remaining GL ES 2.0 entry points ------------------------ */

static bool expect_int(const char *what, long got, long want)
{
	pr_info("gl_test:   %-28s = %ld  want %ld  %s  [check %d]\n", what, got, want,
		got == want ? "ok" : "MISMATCH", gl_test_check_no + 1);
	return gl_test_check(got == want, (uint32_t)got, (uint32_t)want);
}

static int case_api(void)
{
	struct cuv_vertex v[6];
	GLuint prog, tex_prog, tex, tex2, vbo;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};
	static uint8_t patch[4 * 4 * 4];
	static uint8_t rgb_yellow[3 * 3 * 3];       /* unpack alignment 1: 9-byte rows */
	static uint8_t rgb_white[3 * 12];           /* unpack alignment 4: 12-byte rows */

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	tex_prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog || !tex_prog)
		return -1;

	/* frame 1: disabled arrays read the glVertexAttrib* constants: colour
	 * (0, 1, 1, 1) and uv (1, 1) make the quad cyan */
	glUseProgram(prog);
	for (int i = 0; i < 6; i++)
		cuv_set(&v[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);
	vbo = upload_vbo(v, sizeof(v));
	cuv_attribs();
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glVertexAttrib4f(1, 0.0f, 1.0f, 1.0f, 1.0f);
	glVertexAttrib2f(2, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 1"))
		return -1;
	read_topdown();
	ok &= expect_px("constant attribs: cyan", 32, 32, 0xffffff00, 0);
	{
		GLfloat cur[4] = { 9, 9, 9, 9 };

		glGetVertexAttribfv(1, GL_CURRENT_VERTEX_ATTRIB, cur);
		ok &= expect_int("current attrib 1.g", (long)cur[1], 1);
		ok &= expect_int("current attrib 1.r", (long)cur[0], 0);
	}
	glDeleteBuffers(1, &vbo);

	/* frame 2: glTexSubImage2D with both unpack alignments */
	glUseProgram(tex_prog);
	puv_quad_setup();
	tex = make_quadrant_texture(8);
	for (int i = 0; i < 16; i++) { patch[i * 4] = 255; patch[i * 4 + 1] = 0; patch[i * 4 + 2] = 255; patch[i * 4 + 3] = 255; }
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 4, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, patch);
	for (int i = 0; i < 9; i++) { rgb_yellow[i * 3] = 255; rgb_yellow[i * 3 + 1] = 255; rgb_yellow[i * 3 + 2] = 0; }
	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 5, 5, 3, 3, GL_RGB, GL_UNSIGNED_BYTE, rgb_yellow);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	for (int r = 0; r < 3; r++)
		for (int i = 0; i < 9; i++)
			rgb_white[r * 12 + i] = 255;
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 3, 3, GL_RGB, GL_UNSIGNED_BYTE, rgb_white);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 2"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("sub-image rgba: magenta", 16, 16, 0xffff00ff, 0);
	ok &= expect_px("sub-image rgb align 1: yellow", 52, 12, 0xff00ffff, 0);
	ok &= expect_px("sub-image rgb align 4: white", 12, 52, 0xffffffff, 0);
	ok &= expect_px("untouched quadrant: green", 48, 48, 0xff00ff00, 0);

	/* frame 3: glCopyTexImage2D copies the framebuffer into a new texture,
	 * glCopyTexSubImage2D its bottom-left quarter into the top-right */
	glGenTextures(1, &tex2);
	glBindTexture(GL_TEXTURE_2D, tex2);
	glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, TW, TH, 0);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 3"))
		return -1;
	read_topdown();
	ok &= expect_px("copied: magenta", 16, 16, 0xffff00ff, 0);
	ok &= expect_px("copied: yellow", 52, 12, 0xff00ffff, 0);
	ok &= expect_px("copied: green", 48, 48, 0xff00ff00, 0);
	glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 32, 32, 0, 0, 32, 32);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("draw 4"))
		return -1;
	read_topdown();
	ok &= expect_px("sub-copied: white patch", 48, 16, 0xffffffff, 0);
	ok &= expect_px("sub-copied: red", 60, 4, 0xff0000ff, 0);

	/* queries */
	{
		GLfloat fr[2] = { 9, 9 };
		GLboolean b = GL_FALSE;
		GLint iv = -1, n = -1;
		GLuint sh[2] = { 0, 0 };
		GLsizei cnt = 0, len = 0;
		char src[8];

		glGetFloatv(GL_DEPTH_RANGE, fr);
		ok &= expect_int("depth range far", (long)fr[1], 1);
		glGetBooleanv(GL_DEPTH_WRITEMASK, &b);
		ok &= expect_int("depth writemask", b, GL_TRUE);
		glStencilMask(0xffffffffu);   /* the GL default (case 9 leaves 0xff) */
		glGetIntegerv(GL_STENCIL_WRITEMASK, &iv);
		ok &= expect_int("stencil writemask (all ones)", (long)(GLuint)iv, 0xffffffffL);
		ok &= expect_int("glIsTexture(tex2)", glIsTexture(tex2), GL_TRUE);
		ok &= expect_int("glIsTexture(999)", glIsTexture(999), GL_FALSE);
		ok &= expect_int("glIsProgram(prog)", glIsProgram(prog), GL_TRUE);
		ok &= expect_int("glIsShader(0)", glIsShader(0), GL_FALSE);
		glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &n);
		ok &= expect_int("buffer size", n, (long)(6 * sizeof(struct puv_vertex)));
		glGetAttachedShaders(prog, 2, &cnt, sh);
		ok &= expect_int("attached shaders", cnt, 2);
		ok &= expect_int("attached vs is a shader", glIsShader(sh[0]), GL_TRUE);
		glGetShaderSource(sh[0], sizeof(src), &len, src);
		ok &= expect_int("shader source prefix", (long)src[0], (long)gl_test_vs_src[0]);
		ok &= expect_int("shader source chunk", len, (long)sizeof(src) - 1);
		glGetTexParameteriv(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, &iv);
		ok &= expect_int("tex mag filter", iv, GL_NEAREST);
		glGetShaderPrecisionFormat(GL_FRAGMENT_SHADER, GL_HIGH_FLOAT, (GLint[2]){ 0, 0 }, &iv);
		ok &= expect_int("highp float precision", iv, 23);
	}
	if (!check_gl_error("queries"))
		return -1;
	glDeleteTextures(1, &tex);
	glDeleteTextures(1, &tex2);
	return ok ? 0 : -1;
}

/* ---- case 15: EGL (pbuffer surface, queries, context lifetime) ---------- */

static int case_egl(void)
{
	static const EGLint cfg_attribs[] = {
		EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_ALPHA_SIZE, 8, EGL_NONE,
	};
	static const EGLint bad_attribs[] = { EGL_SAMPLES, 8, EGL_NONE };
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLDisplay dpy;
	EGLConfig cfg;
	EGLSurface pb;
	EGLContext ctx;
	EGLint n = 0, v = 0;
	struct cuv_vertex tri[3];
	GLuint prog;
	bool ok = true;

	/* the harness context goes away: the EGL context replaces it */
	dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	ok &= expect_int("eglInitialize", eglInitialize(dpy, NULL, NULL), EGL_TRUE);
	ok &= expect_int("eglChooseConfig", eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n), EGL_TRUE);
	ok &= expect_int("one config", n, 1);
	ok &= expect_int("no 8x MSAA config", eglChooseConfig(dpy, bad_attribs, NULL, 0, &n) && n == 0, 1);
	eglGetConfigAttrib(dpy, cfg, EGL_DEPTH_SIZE, &v);
	ok &= expect_int("config depth size", v, 24);
	ok &= expect_int("eglQueryString VENDOR", eglQueryString(dpy, EGL_VENDOR) != NULL, 1);
	pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	ok &= expect_int("pbuffer created", pb != EGL_NO_SURFACE, 1);
	ok &= expect_int("context created", ctx != EGL_NO_CONTEXT, 1);
	ok &= expect_int("eglMakeCurrent", eglMakeCurrent(dpy, pb, pb, ctx), EGL_TRUE);
	ok &= expect_int("current context", eglGetCurrentContext() == ctx, 1);
	eglQuerySurface(dpy, pb, EGL_WIDTH, &v);
	ok &= expect_int("pbuffer width", v, TW);
	ok &= expect_int("eglGetProcAddress", eglGetProcAddress("glDrawArrays") ==
			 (__eglMustCastToProperFunctionPointerType)glDrawArrays, 1);
	if (!ok)
		return -1;

	/* the slice-A triangle into the pbuffer, read back through GL */
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	slice_a_vertices(tri, 0);
	upload_vbo(tri, sizeof(tri));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	ok &= expect_int("eglSwapBuffers", eglSwapBuffers(dpy, pb), EGL_TRUE);
	if (!check_gl_error("draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("egl pbuffer: centroid", 32, 40, A_CENTROID, 3);
	ok &= expect_px("egl pbuffer: background", 1, 1, 0xff000000, 0);

	ok &= expect_int("release", eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT), EGL_TRUE);
	ok &= expect_int("no current context", eglGetCurrentContext() == EGL_NO_CONTEXT, 1);
	ok &= expect_int("eglDestroySurface", eglDestroySurface(dpy, pb), EGL_TRUE);
	ok &= expect_int("eglDestroyContext", eglDestroyContext(dpy, ctx), EGL_TRUE);
	ok &= expect_int("eglTerminate", eglTerminate(dpy), EGL_TRUE);
	ok &= expect_int("eglGetError clean", eglGetError(), EGL_SUCCESS);
	return ok ? 0 : -1;
}

/* ---- case 16: 4x multisampling (EGL_SAMPLES 4 pbuffer) ------------------ */

/* A magenta triangle whose hypotenuse is the window diagonal x = y: every
 * pixel on it is exactly half covered. With 4 samples at (6,2) (14,6) (2,10)
 * (10,14)/16, two samples fall on each side, so the resolved pixel is half
 * magenta; single-sampled it is either full or empty. */
#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
/* Which 256-byte chunks of the 4x scratch surface a frame wrote, 64 to a
 * line ('#' written, '.' not), so where the writes landed is visible. */
static void msaa_print_map(const char *what)
{
	extern int gc_debug_msaa_scratch_map(uint8_t *out, uint32_t n);
	static uint8_t map[1024];
	int n = gc_debug_msaa_scratch_map(map, sizeof(map));
	char line[65];

	if (n <= 0)
		return;
	pr_info("gl_test:   x4 %s map, %d chunks of 256 bytes:\n", what, n);
	for (int i = 0; i < n && i < (int)sizeof(map); i += 64) {
		int k;

		for (k = 0; k < 64 && i + k < n; k++)
			line[k] = map[i + k] ? '#' : '.';
		line[k] = 0;
		pr_info("gl_test:     %4d %s\n", i, line);
	}
}
#endif

static int case_msaa_at(EGLDisplay dpy, EGLConfig cfg, int samples, bool *ok)
{
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLSurface pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	struct cuv_vertex tri[3];
	GLuint prog;
	GLint v = -1;
	uint32_t edge, in, out;

	if (pb == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, pb, pb, ctx)) {
		pr_err("gl_test: msaa x%d: surface/context failed (0x%x)\n", samples, eglGetError());
		return -1;
	}
	glGetIntegerv(GL_SAMPLES, &v);
	*ok &= expect_int(samples > 1 ? "GL_SAMPLES x4" : "GL_SAMPLES x1", v, samples > 1 ? samples : 0);
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	cuv_set(&tri[0], -1.0f, -1.0f, 0.0f, 1, 0, 1);
	cuv_set(&tri[1], 1.0f, -1.0f, 0.0f, 1, 0, 1);
	cuv_set(&tri[2], 1.0f, 1.0f, 0.0f, 1, 0, 1);
	upload_vbo(tri, sizeof(tri));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	eglSwapBuffers(dpy, pb);
	if (!check_gl_error("msaa draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	/* window (x, y) bottom-up: the diagonal is x == y; top-down row = TH-1-y */
	in = s_px[(TH - 1 - 20) * TW + 40];
	out = s_px[(TH - 1 - 40) * TW + 20];
	edge = s_px[(TH - 1 - 32) * TW + 32];
	pr_info("gl_test:   x%d: inside 0x%08x outside 0x%08x edge 0x%08x\n", samples, in, out, edge);
#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
	/* Which half of a failed 4x frame failed: samples in the scratch and a
	 * blank target is the resolve's; an empty scratch is the draw's. The
	 * resolve leaves the samples in place, so this reads after it. */
	if (samples > 1) {
		extern int gc_debug_msaa_scratch(uint32_t *out);
		uint32_t sc[4];

		if (gc_debug_msaa_scratch(sc) == 0)
			pr_info("gl_test:   x4 scratch: %u bytes, %u non-zero dwords, first [%u] = 0x%08x\n",
				sc[0], sc[1], sc[2], sc[3]);
		else
			pr_info("gl_test:   x4 scratch: none allocated (the draw did not take the 4x path)\n");
	}
	/* Then the clear and the draw apart, each into an emptied scratch, so a
	 * half-written scratch names which of the two did not land. Prints
	 * only: the verdict above is the case's. */
	if (samples > 1) {
		extern int gc_debug_msaa_scratch(uint32_t *out);
		extern int gc_debug_msaa_scratch_zero(void);
		uint32_t sc[4];

		if (gc_debug_msaa_scratch_zero() == 0) {
			glClear(GL_COLOR_BUFFER_BIT);
			glFinish();
			if (gc_debug_msaa_scratch(sc) == 0)
				pr_info("gl_test:   x4 clear only: %u non-zero dwords of %u, first [%u] = 0x%08x\n",
					sc[1], sc[0] / 4, sc[2], sc[3]);
			msaa_print_map("clear only");
		}
		if (gc_debug_msaa_scratch_zero() == 0) {
			glDrawArrays(GL_TRIANGLES, 0, 3);
			glFinish();
			if (gc_debug_msaa_scratch(sc) == 0)
				pr_info("gl_test:   x4 draw only: %u non-zero dwords of %u, first [%u] = 0x%08x\n",
					sc[1], sc[0] / 4, sc[2], sc[3]);
			msaa_print_map("draw only");
		}
	}
#endif
	*ok &= expect_px(samples > 1 ? "x4 inside" : "x1 inside", 40, TH - 1 - 20, 0xffff00ff, 0);
	*ok &= expect_px(samples > 1 ? "x4 outside" : "x1 outside", 20, TH - 1 - 40, 0xff000000, 0);
	if (samples > 1)
		*ok &= expect_px("x4 edge: half covered", 32, TH - 1 - 32, 0xff800080, 2);
	else
		*ok &= expect_int("x1 edge: full or empty", (edge & 0xff) == 0xff || (edge & 0xff) == 0, 1);
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, pb);
	eglDestroyContext(dpy, ctx);
	return 0;
}

/* ---- case 145: glSampleCoverage on a 4x surface ---------------------------
 *
 * ES 2.0.25 4.1.3: with SAMPLE_COVERAGE on, the fragment coverage is ANDed
 * with a temporary coverage made from SAMPLE_COVERAGE_VALUE, inverted with
 * SAMPLE_COVERAGE_INVERT. The GL core gives the backend Mesa's mask (the
 * first floor(value x 4) samples; draw.rs set_sample_state), so each phase's
 * resolved colour is the fraction of samples drawn: red at 0.5 then green at
 * 0.5 inverted fill the two halves of the mask, one colour each (0x80, 0x80,
 * 0); red at 0.25 is one sample (0x40); at 0.0 none; coverage off, all four.
 * A backend that ignored the mask draws full red and then full green (A
 * reads green), full red for B and C. Pixels in 0xAABBGGRR. */
static int samplecov_phase(const char *name, float value, bool invert, bool on, uint32_t want,
			   bool green_after)
{
	static const struct { int x, y; } at[3] = { { 8, 8 }, { 32, 32 }, { 55, 57 } };
	bool ok = true;

	glDisable(GL_SAMPLE_COVERAGE);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	if (on)
		glEnable(GL_SAMPLE_COVERAGE);
	glSampleCoverage(value, invert ? GL_TRUE : GL_FALSE);
	glDrawArrays(GL_TRIANGLES, 0, 6); /* red */
	if (green_after) {
		glSampleCoverage(value, GL_TRUE);
		glDrawArrays(GL_TRIANGLES, 6, 6); /* green on the other samples */
	}
	glDisable(GL_SAMPLE_COVERAGE);
	if (!check_gl_error(name))
		return -1;
	read_topdown();
	for (int i = 0; i < 3; i++) {
		char what[64];

		snprintf(what, sizeof(what), "samplecov %s (%d,%d)", name, at[i].x, at[i].y);
		ok &= expect_px(what, at[i].x, TH - 1 - at[i].y, want, 2);
	}
	return ok ? 0 : -1;
}

static int case_samplecov(void)
{
	static const EGLint cfg4[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, 4, EGL_NONE };
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLint n = 0;
	struct cuv_vertex q[12];
	GLuint prog;
	int rc = 0;

	if (!eglInitialize(dpy, NULL, NULL))
		return -1;
	if (!eglChooseConfig(dpy, cfg4, &cfg, 1, &n) || n != 1) {
		pr_info("gl_test: case 145 samplecov: SKIP (no 4x EGL config offered)\n");
		return -3;
	}
	EGLSurface pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (pb == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, pb, pb, ctx)) {
		pr_err("gl_test: samplecov: surface/context failed (0x%x)\n", eglGetError());
		return -1;
	}
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		cuv_set(&q[i], c[i][0], c[i][1], 0.0f, 1, 0, 0);     /* red */
		cuv_set(&q[6 + i], c[i][0], c[i][1], 0.0f, 0, 1, 0); /* green */
	}
	upload_vbo(q, sizeof(q));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	if (samplecov_phase("A: red 0.5, green 0.5 inverted", 0.5f, false, true, 0xff008080u, true) != 0)
		rc = -1;
	if (samplecov_phase("B: red 0.25", 0.25f, false, true, 0xff000040u, false) != 0)
		rc = -1;
	if (samplecov_phase("C: red 0.0", 0.0f, false, true, 0xff000000u, false) != 0)
		rc = -1;
	if (samplecov_phase("D: red, coverage off", 0.5f, false, false, 0xff0000ffu, false) != 0)
		rc = -1;
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, pb);
	eglDestroyContext(dpy, ctx);
	return rc;
}

static int case_msaa(void)
{
	static const EGLint cfg4[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, 4, EGL_NONE };
	static const EGLint cfg1[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_NONE };
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLint n = 0, v = 0;
	bool ok = true;

	if (!eglInitialize(dpy, NULL, NULL))
		return -1;
#if defined(GL_TEST_VIVANTE) || defined(GL_TEST_V3D)
	/* These two backends offer the 4x config only while their MSAA switch is
	 * on (vivante::backend::msaa_on, CONFIG_GC_GL_TEST_MSAA; the V3D's
	 * CONFIG_V3D_GL_TEST_MSAA), off in products and in the default image.
	 * Without it "eglChooseConfig samples 4" can only read 0, which says
	 * nothing about the draw (#8's "check 255" is the record's none marker,
	 * not a check): the case runs its 1x half and is skipped (neither a
	 * pass nor a failure), as case 145 is. The PowerVR always offers 4x,
	 * so there a missing config stays a failure. */
	const bool have_4x = eglChooseConfig(dpy, cfg4, &cfg, 1, &n) && n == 1;
#else
	const bool have_4x = true;
#endif

	if (have_4x) {
		ok &= expect_int("eglChooseConfig samples 4", eglChooseConfig(dpy, cfg4, &cfg, 1, &n) && n == 1, 1);
		eglGetConfigAttrib(dpy, cfg, EGL_SAMPLES, &v);
		ok &= expect_int("config EGL_SAMPLES", v, 4);
		if (!ok || case_msaa_at(dpy, cfg, 4, &ok) != 0)
			return -1;
	}
	ok &= expect_int("eglChooseConfig default", eglChooseConfig(dpy, cfg1, &cfg, 1, &n) && n == 1, 1);
	eglGetConfigAttrib(dpy, cfg, EGL_SAMPLES, &v);
	ok &= expect_int("default config is single-sampled", v, 0);
	if (!ok || case_msaa_at(dpy, cfg, 1, &ok) != 0)
		return -1;
	eglTerminate(dpy);
	if (!have_4x) {
		pr_info("gl_test: case 16 msaa: 1x half done; SKIP the 4x half (no 4x EGL config offered)\n");
		return ok ? -3 : -1;
	}
	return ok ? 0 : -1;
}

/* The bring-up context gl_test_run creates before the cases; the EGL cases
 * that need every pool slot release it (it is not used afterwards). */
static struct gl_context *s_harness_ctx;

static void release_harness_context(void)
{
	if (s_harness_ctx) {
		gl_context_destroy(s_harness_ctx);
		s_harness_ctx = NULL;
	}
}

/* ---- case 17: two EGL contexts with their own objects and state ---------- */

/* How many contexts case 17 will try before calling a pool that never refuses
 * a defect. Above any pool this project builds (2 by default, 32 in a
 * conformance pair) with room, and bounded so that "it never refuses" is a
 * reported failure rather than a hang nobody can read. */
#define GL_TEST_CTX_PROBE_MAX 64

static int case_ctx2(void)
{
	static const EGLint cfg_attribs[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_NONE };
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLSurface pa, pb;
	EGLContext ca, cb, extra[GL_TEST_CTX_PROBE_MAX];
	int made = 0, refused_at = -1, refuse_err = 0, i;
	EGLint n = 0;
	GLuint prog_a, prog_b, tex_a;
	GLfloat cc4[4] = { 9, 9, 9, 9 };
	struct cuv_vertex v[6];
	static const float q[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};
	bool ok = true;

	/* the harness's bring-up context holds one of the GL_MAX_CONTEXTS pool
	 * slots; this case needs both (it runs after the other EGL cases) */
	release_harness_context();
	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1)
		return -1;
	pa = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	ca = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	cb = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	ok &= expect_int("two contexts created", ca != EGL_NO_CONTEXT && cb != EGL_NO_CONTEXT, 1);

	/* The pool is finite and refuses cleanly past its limit -- but the limit
	 * is a build setting (YAKOGL_MAX_CONTEXTS), not a constant. It is 2 by
	 * default and 32 in a conformance pair, because dEQP's multi_context
	 * needs three. This used to assert that the THIRD context was refused,
	 * which made the case fail on the conformance pair for a reason that has
	 * nothing to do with the GPU: measured on one HEAD, 31 of 38 at the
	 * default pool and 30 of 38 at 32/32, the single difference being this
	 * case.
	 *
	 * So the limit is discovered rather than assumed: keep creating until one
	 * is refused, and check what matters at any pool size -- that the refusal
	 * comes, that it is EGL_BAD_ALLOC and not some other error, and that the
	 * pool held at least the two this case goes on to use. A cap, because a
	 * pool that never refuses is itself the defect and an unbounded loop
	 * would hide it as a hang. */
	for (i = 0; i < GL_TEST_CTX_PROBE_MAX; i++) {
		extra[i] = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
		if (extra[i] == EGL_NO_CONTEXT) {
			refuse_err = eglGetError();
			refused_at = made;
			break;
		}
		made++;
	}
	ok &= expect_int("the context pool refuses past its limit", refused_at >= 0, 1);
	ok &= expect_int("and refuses with EGL_BAD_ALLOC", refuse_err == EGL_BAD_ALLOC, 1);
	for (i = 0; i < made; i++)
		eglDestroyContext(dpy, extra[i]);

	/* context A: red clear colour, the slice-A triangle, a texture */
	ok &= expect_int("makeCurrent A", eglMakeCurrent(dpy, pa, pa, ca), EGL_TRUE);
	prog_a = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog_a)
		return -1;
	glUseProgram(prog_a);
	tex_a = make_quadrant_texture(4);
	slice_a_vertices(v, 0);
	upload_vbo(v, sizeof(v[0]) * 3);
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	eglSwapBuffers(dpy, pa);
	read_topdown();
	ok &= expect_px("A: centroid", 32, 40, A_CENTROID, 3);
	ok &= expect_px("A: red background", 1, 1, 0xff0000ff, 0);

	/* context B: its own objects and default state; a blue quad */
	ok &= expect_int("makeCurrent B", eglMakeCurrent(dpy, pb, pb, cb), EGL_TRUE);
	ok &= expect_int("B: A's program is unknown", glIsProgram(prog_a), GL_FALSE);
	ok &= expect_int("B: A's texture is unknown", glIsTexture(tex_a), GL_FALSE);
	glGetFloatv(GL_COLOR_CLEAR_VALUE, cc4);
	ok &= expect_int("B: default clear colour", cc4[0] == 0.0f && cc4[3] == 0.0f, 1);
	prog_b = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog_b)
		return -1;
	glUseProgram(prog_b);
	for (int i = 0; i < 6; i++)
		cuv_set(&v[i], q[i][0], q[i][1], 0.0f, 0, 0, 1);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	eglSwapBuffers(dpy, pb);
	read_topdown();
	ok &= expect_px("B: blue quad", 32, 32, 0xffff0000, 0);

	/* back to A: state and objects intact, its surface untouched */
	ok &= expect_int("makeCurrent A again", eglMakeCurrent(dpy, pa, pa, ca), EGL_TRUE);
	ok &= expect_int("A: program still there", glIsProgram(prog_a), GL_TRUE);
	ok &= expect_int("A: texture still there", glIsTexture(tex_a), GL_TRUE);
	glGetFloatv(GL_COLOR_CLEAR_VALUE, cc4);
	ok &= expect_int("A: clear colour kept", cc4[0] == 1.0f && cc4[1] == 0.0f, 1);
	read_topdown();
	ok &= expect_px("A: surface kept", 32, 40, A_CENTROID, 3);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	eglSwapBuffers(dpy, pa);
	read_topdown();
	ok &= expect_px("A: redraw with its program", 32, 40, A_CENTROID, 3);
	if (!check_gl_error("ctx2"))
		return -1;

	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroyContext(dpy, ca);
	eglDestroyContext(dpy, cb);
	eglDestroySurface(dpy, pa);
	eglDestroySurface(dpy, pb);
	eglTerminate(dpy);
	return ok ? 0 : -1;
}

/* ---- case 18: EGLImage (GL texture and native buffer sources) ----------- */

static int case_image(void)
{
	static const EGLint cfg_attribs[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_NONE };
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window nb;
	static const uint8_t stripes[4 * 4 * 4] = {   /* rows: red, green, blue, white (RGBA) */
		255,0,0,255, 255,0,0,255, 255,0,0,255, 255,0,0,255,
		0,255,0,255, 0,255,0,255, 0,255,0,255, 0,255,0,255,
		0,0,255,255, 0,0,255,255, 0,0,255,255, 0,0,255,255,
		255,255,255,255, 255,255,255,255, 255,255,255,255, 255,255,255,255,
	};
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLSurface pa, pb;
	EGLContext ca, cb;
	EGLImage img_tex, img_nb;
	EGLint n = 0;
	GLuint tex_a, tex_b, tex_n, prog;
	struct osal_gpu_mem nbmem;
	bool ok = true;

	release_harness_context();
	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1)
		return -1;
	ok &= expect_int("EGL_KHR_image_base advertised",
			 strstr(eglQueryString(dpy, EGL_EXTENSIONS), "EGL_KHR_image_base") != NULL, 1);
	pa = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	ca = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	cb = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (!eglMakeCurrent(dpy, pa, pa, ca))
		return -1;

	/* context A owns a quadrant texture; an image is made from it */
	tex_a = make_quadrant_texture(8);
	img_tex = eglCreateImage(dpy, ca, EGL_GL_TEXTURE_2D, (EGLClientBuffer)(uintptr_t)tex_a, NULL);
	ok &= expect_int("image from GL texture", img_tex != EGL_NO_IMAGE, 1);
	ok &= expect_int("image from a bad texture fails",
			 eglCreateImage(dpy, ca, EGL_GL_TEXTURE_2D, (EGLClientBuffer)(uintptr_t)999, NULL) == EGL_NO_IMAGE &&
			 eglGetError() == EGL_BAD_PARAMETER, 1);

	/* a native buffer (pool memory here; a camera frame or scan-out buffer
	 * in an application) filled with four coloured rows */
	if (osal_gpu_alloc(yakogl_heap_base(GLES_HEAP_DATA), sizeof(stripes), OSAL_MEM_DATA, &nbmem) != 0)
		return -1;
	if (nbmem.cpu) {
		memcpy(nbmem.cpu, stripes, sizeof(stripes));
		if (g_osal->cache_flush)
			g_osal->cache_flush(nbmem.cpu, sizeof(stripes));
	} else {
		g_osal->gpu_write(&nbmem, 0, stripes, sizeof(stripes));
	}
	nb.phys = nbmem.phys;
	nb.width = 4;
	nb.height = 4;
	nb.stride_bytes = 16;
	nb.format = GLES_NATIVE_FORMAT_RGBA8888;
	img_nb = eglCreateImage(dpy, EGL_NO_CONTEXT, EGL_NATIVE_PIXMAP_KHR, (EGLClientBuffer)&nb, NULL);
	ok &= expect_int("image from native buffer", img_nb != EGL_NO_IMAGE, 1);
	if (!ok)
		return -1;

	/* context B samples both images through its own textures */
	if (!eglMakeCurrent(dpy, pb, pb, cb))
		return -1;
	ok &= expect_int("GL_OES_EGL_image advertised",
			 strstr((const char *)glGetString(GL_EXTENSIONS), "GL_OES_EGL_image") != NULL, 1);
	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glGenTextures(1, &tex_b);
	glBindTexture(GL_TEXTURE_2D, tex_b);
	glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, (GLeglImageOES)img_tex);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	eglSwapBuffers(dpy, pb);
	if (!check_gl_error("image texture draw"))
		return -1;
	read_topdown();
	ok &= expect_px("texture image: top-left blue", 16, 16, 0xffff0000, 0);
	ok &= expect_px("texture image: bottom-right green", 48, 48, 0xff00ff00, 0);

	glGenTextures(1, &tex_n);
	glBindTexture(GL_TEXTURE_2D, tex_n);
	glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, (GLeglImageOES)img_nb);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	eglSwapBuffers(dpy, pb);
	if (!check_gl_error("native image draw"))
		return -1;
	read_topdown();
	/* texel row 0 (red) is t = 0: the bottom of the screen */
	ok &= expect_px("native image: bottom row red", 32, 60, 0xff0000ff, 0);
	ok &= expect_px("native image: top row white", 32, 4, 0xffffffff, 0);
	ok &= expect_px("native image: second row green", 32, 44, 0xff00ff00, 0);

	/* the image outlives the texture that created it in A */
	if (!eglMakeCurrent(dpy, pa, pa, ca))
		return -1;
	glDeleteTextures(1, &tex_a);
	if (!eglMakeCurrent(dpy, pb, pb, cb))
		return -1;
	glBindTexture(GL_TEXTURE_2D, tex_b);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	eglSwapBuffers(dpy, pb);
	read_topdown();
	ok &= expect_px("image kept after source delete", 16, 16, 0xffff0000, 0);

	ok &= expect_int("eglDestroyImage", eglDestroyImage(dpy, img_tex) && eglDestroyImage(dpy, img_nb), 1);
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroyContext(dpy, ca);
	eglDestroyContext(dpy, cb);
	eglDestroySurface(dpy, pa);
	eglDestroySurface(dpy, pb);
	eglTerminate(dpy);
	/* the native buffer goes back to the pool once nothing uses it (the
	 * owner report counted it as unaccounted: 4 KiB at every close) */
	if (g_osal->gpu_free)
		g_osal->gpu_free(&nbmem);
	return ok ? 0 : -1;
}

/* ---- case 19: GLSL ES 1.00 shaders, as OpenGL ES 2.0 applications write them ---- */

/* The PowerVR SDK HelloAPI flow: attribute bound by name before the link, a
 * 3-float position array, the mat4 uniform by name. */
static int case_es100(void)
{
	static const float hello_verts[] = { -0.4f, -0.4f, 0.0f, 0.4f, -0.4f, 0.0f, 0.0f, 0.4f, 0.0f };
	static const float identity[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 };
	GLuint vs, fs, prog, vbo, tex, tprog;
	GLint loc, ok_link = 0, l_pos, l_uv, l_tint, l_mvp, l_tex, l_scale;
	const char *src;
	struct puv_vertex q[6];
	bool ok = true;

	vs = glCreateShader(GL_VERTEX_SHADER);
	src = gl_test_es100_hello_vs_src;
	glShaderSource(vs, 1, &src, NULL);
	glCompileShader(vs);
	fs = glCreateShader(GL_FRAGMENT_SHADER);
	src = gl_test_es100_hello_fs_src;
	glShaderSource(fs, 1, &src, NULL);
	glCompileShader(fs);
	prog = glCreateProgram();
	glAttachShader(prog, fs);
	glAttachShader(prog, vs);
	glBindAttribLocation(prog, 0, "myVertex");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok_link);
	ok &= expect_int("HelloAPI program links", ok_link, GL_TRUE);
	if (!ok_link) {
		char log[128];

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: link log: %s\n", log);
		return -1;
	}
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "transformationMatrix");
	ok &= expect_int("transformationMatrix found", loc >= 0, 1);
	glUniformMatrix4fv(loc, 1, GL_FALSE, identity);
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(hello_verts), hello_verts, GL_STATIC_DRAW);
	glEnableVertexAttribArray(0);
	glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 0, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glClearColor(0.00f, 0.70f, 0.67f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("hello"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("HelloAPI: triangle (1, 1, 0.66)", 32, 36, 0xffa8ffff, 1);
	ok &= expect_px("HelloAPI: clear (0, 0.7, 0.67)", 4, 4, 0xffabb300, 2);
	glDeleteBuffers(1, &vbo);

	/* an IntroducingPVRShell-style textured pair: attributes and uniforms
	 * looked up by name, varyings declared in different orders in the two
	 * shaders (linked by name), a constant attribute for the tint */
	tprog = link_program(gl_test_es100_tex_vs_src, gl_test_es100_tex_fs_src);
	if (!tprog)
		return -1;
	glUseProgram(tprog);
	l_pos = glGetAttribLocation(tprog, "inVertex");
	l_uv = glGetAttribLocation(tprog, "inTexCoord");
	l_tint = glGetAttribLocation(tprog, "inTint");
	l_mvp = glGetUniformLocation(tprog, "MVPMatrix");
	l_tex = glGetUniformLocation(tprog, "sTexture");
	l_scale = glGetUniformLocation(tprog, "uScale");
	ok &= expect_int("attribute locations by name", l_pos == 0 && l_uv == 1 && l_tint == 2, 1);
	ok &= expect_int("uniform locations by name", l_mvp >= 0 && l_tex >= 0 && l_scale >= 0, 1);
	glUniformMatrix4fv(l_mvp, 1, GL_FALSE, identity);
	glUniform1i(l_tex, 0);
	glUniform4f(l_scale, 1.0f, 1.0f, 1.0f, 1.0f);
	tex = make_quadrant_texture(8);
	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer((GLuint)l_pos, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glVertexAttribPointer((GLuint)l_uv, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
	glEnableVertexAttribArray((GLuint)l_pos);
	glEnableVertexAttribArray((GLuint)l_uv);
	glDisableVertexAttribArray((GLuint)l_tint);
	glVertexAttrib4f((GLuint)l_tint, 1.0f, 1.0f, 1.0f, 1.0f);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("textured"))
		return -1;
	read_topdown();
	ok &= expect_px("es100 textured: top-left blue", 16, 16, 0xffff0000, 0);
	ok &= expect_px("es100 textured: top-right white", 48, 16, 0xffffffff, 0);
	ok &= expect_px("es100 textured: bottom-left red", 16, 48, 0xff0000ff, 0);
	/* the tint attribute keeps only green; uScale halves it */
	glVertexAttrib4f((GLuint)l_tint, 0.0f, 1.0f, 0.0f, 1.0f);
	glUniform4f(l_scale, 1.0f, 0.5f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	read_topdown();
	ok &= expect_px("es100 tint+scale: top-right half green", 48, 16, 0xff008000, 1);
	ok &= expect_px("es100 tint+scale: bottom-left black", 16, 48, 0xff000000, 0);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 20: ETC1 / ETC2 compressed textures (TPU decoded) ------------- */

/* A flat ETC1 block: individual mode, both halves the same 4-bit colour,
 * modifier table 0 and every pixel index 0 (+2): colour = c * 17 + 2. */
static void etc1_flat(uint8_t *b, unsigned r, unsigned g, unsigned bl)
{
	b[0] = (uint8_t)((r << 4) | r);
	b[1] = (uint8_t)((g << 4) | g);
	b[2] = (uint8_t)((bl << 4) | bl);
	b[3] = 0;   /* tables 0, no diff, no flip */
	b[4] = b[5] = b[6] = b[7] = 0;
}

static uint32_t etc1_colour(unsigned r, unsigned g, unsigned bl)
{
	uint32_t R = r * 17 + 2 > 255 ? 255 : r * 17 + 2;
	uint32_t G = g * 17 + 2 > 255 ? 255 : g * 17 + 2;
	uint32_t B = bl * 17 + 2 > 255 ? 255 : bl * 17 + 2;

	return 0xff000000u | (B << 16) | (G << 8) | R;
}

static int case_etc(void)
{
	/* 16x8 ETC1: 4x2 blocks, each a different flat colour (the twiddled
	 * block order shows if any block lands in the wrong place) */
	static const unsigned cols[8][3] = {
		{ 15, 0, 0 }, { 0, 15, 0 }, { 0, 0, 15 }, { 15, 15, 15 },
		{ 15, 15, 0 }, { 0, 15, 15 }, { 15, 0, 15 }, { 8, 8, 8 },
	};
	static uint8_t etc1[8 * 8];
	static uint8_t etc2a[4 * 16];
	static const uint8_t alphas[4] = { 255, 128, 64, 0 };
	GLuint prog, tex;
	GLint n = 0, fmts[4] = { 0, 0, 0, 0 };
	bool ok = true;

	glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &n);
	ok &= expect_int("3 compressed formats", n, 3);
	glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, fmts);
	ok &= expect_int("ETC1 listed", fmts[0], GL_ETC1_RGB8_OES);
	ok &= expect_int("GL_OES_compressed_ETC1_RGB8_texture advertised",
			 strstr((const char *)glGetString(GL_EXTENSIONS), "GL_OES_compressed_ETC1_RGB8_texture") != NULL, 1);

	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);

	for (int by = 0; by < 2; by++)
		for (int bx = 0; bx < 4; bx++)
			etc1_flat(&etc1[(by * 4 + bx) * 8], cols[by * 4 + bx][0], cols[by * 4 + bx][1], cols[by * 4 + bx][2]);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 16, 8, 0, sizeof(etc1), etc1);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("etc1 upload"))
		return -1;
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("etc1 draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	/* block row 0 is t = 0: the bottom half of the screen */
	for (int by = 0; by < 2; by++) {
		for (int bx = 0; bx < 4; bx++) {
			char what[GL_TEST_LABEL];

			snprintf(what, sizeof(what), "etc1 block (%d,%d)", bx, by);
			ok &= expect_px(what, (uint32_t)(bx * 16 + 8), (uint32_t)(by == 0 ? 48 : 16),
					etc1_colour(cols[by * 4 + bx][0], cols[by * 4 + bx][1], cols[by * 4 + bx][2]), 1);
		}
	}
	glDeleteTextures(1, &tex);

	/* 8x8 ETC2 RGBA8 (EAC alpha + ETC2 colour): four blocks with alpha
	 * 255/128/64/0 over red/green/blue/white */
	for (int i = 0; i < 4; i++) {
		uint8_t *blk = &etc2a[i * 16];

		memset(blk, 0, 16);
		blk[0] = alphas[i];   /* EAC: base alpha, multiplier 0, table 0, indices 0 */
		etc1_flat(blk + 8, cols[i][0], cols[i][1], cols[i][2]);
	}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x9278 /* GL_COMPRESSED_RGBA8_ETC2_EAC (gl3.h) */, 8, 8, 0, sizeof(etc2a), etc2a);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	if (!check_gl_error("etc2 upload"))
		return -1;
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("etc2 draw"))
		return -1;
	read_topdown();
	for (int i = 0; i < 4; i++) {
		char what[GL_TEST_LABEL];
		uint32_t want = (etc1_colour(cols[i][0], cols[i][1], cols[i][2]) & 0x00ffffffu) |
				((uint32_t)alphas[i] << 24);

		snprintf(what, sizeof(what), "etc2 rgba block %d", i);
		ok &= expect_px(what, (uint32_t)((i & 1) * 32 + 16), (uint32_t)((i >> 1) == 0 ? 48 : 16), want, 1);
	}
	/* a bad image size and an unknown format are rejected */
	glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_ETC1_RGB8_OES, 8, 8, 0, 8, etc1);
	ok &= expect_int("wrong imageSize -> INVALID_VALUE", glGetError(), GL_INVALID_VALUE);
	glCompressedTexImage2D(GL_TEXTURE_2D, 0, 0x8C00, 8, 8, 0, 32, etc1);   /* PVRTC 4bpp */
	ok &= expect_int("PVRTC -> INVALID_ENUM", glGetError(), GL_INVALID_ENUM);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 126: an ETC1 mip chain ------------------------------------------
 * 16x16 ETC1 with all five levels (4x4, 2x2, 1, 1 and 1 blocks), each level
 * one flat colour. The full-screen quad drawn into a (16 >> l)-square
 * viewport minifies by 2^l, so NEAREST_MIPMAP_NEAREST reads level l; every
 * pixel of the viewport must be that level's colour. The PowerVR sampled an
 * ETC texture's level 0 only and wrote the level count into the twiddled
 * IMAGE_WORD1's baselevel (dEQP filtering.2d *_mipmap_*_etc1 on the AM62P);
 * the GC7000 reads the same chain from its row layout. */
static int case_etcmip(void)
{
	static const unsigned cols[5][3] = {
		{ 15, 0, 0 }, { 0, 15, 0 }, { 0, 0, 15 }, { 15, 15, 15 }, { 15, 15, 0 },
	};
	static uint8_t lvl[16 * 8];
	GLuint prog, tex;
	bool ok = true;

	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	for (int l = 0; l < 5; l++) {
		int side = 16 >> l, blocks = ((side + 3) / 4) * ((side + 3) / 4);

		for (int b = 0; b < blocks; b++)
			etc1_flat(&lvl[b * 8], cols[l][0], cols[l][1], cols[l][2]);
		glCompressedTexImage2D(GL_TEXTURE_2D, l, GL_ETC1_RGB8_OES, side, side, 0, blocks * 8, lvl);
	}
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("etcmip upload"))
		return -1;
	for (int l = 0; l < 5; l++) {
		uint32_t side = 16u >> l, want = etc1_colour(cols[l][0], cols[l][1], cols[l][2]);
		uint32_t right = 0;

		glViewport(0, 0, (GLsizei)side, (GLsizei)side);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("etcmip draw"))
			return -1;
		read_topdown();
		/* the viewport is the bottom-left corner: rows TH - side .. TH - 1 */
		for (uint32_t y = TH - side; y < TH; y++)
			for (uint32_t x = 0; x < side; x++)
				right += chan_diff(s_px[y * TW + x], want) <= 1;
		if (right != side * side)
			pr_info("gl_test: etcmip level %d: %u of %u pixels right, (0,%u) = %08x want %08x\n", l,
				right, side * side, TH - 1, s_px[(TH - 1) * TW], want);
		ok &= gl_test_check(right == side * side, right, side * side);
	}
	glViewport(0, 0, TW, TH);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 10: primitives, client-side arrays, index types ---------------- */

/* ---- case 56: many lines in one draw ------------------------------------
 * Qt's GridGeometry submits forty line primitives in a single glDrawArrays and
 * the board draws four of them, while case 3's two lines have always worked.
 * This walks the count -- 2, 4, 8, 16, 20 evenly spaced horizontal lines in one
 * draw -- and reports how many rows actually lit, which separates "lines are
 * broken" from "many lines in one draw are broken".
 */
static int case_manylines(void)
{
	static struct cuv_vertex v[64];
	GLuint prog;
	bool ok = true;
	static const int counts[] = { 2, 4, 8, 16, 20 };

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), v[0].pos);
	glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), v[0].uv);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(2);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_CULL_FACE);

	for (unsigned c = 0; c < sizeof(counts) / sizeof(counts[0]); c++) {
		const int n = counts[c];
		int lit = 0;

		for (int i = 0; i < n; i++) {
			const uint32_t row = (uint32_t)(2 + i * 3);

			cuv_set(&v[i * 2 + 0], ndc_x(4), ndc_y(row), 0.0f, 1, 1, 1);
			cuv_set(&v[i * 2 + 1], ndc_x(59), ndc_y(row), 0.0f, 1, 1, 1);
		}
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_LINES, 0, n * 2);
		glFinish();
		if (!check_gl_error("many lines"))
			return -1;
		read_topdown();
		for (int i = 0; i < n; i++) {
			const uint32_t row = (uint32_t)(2 + i * 3);

			if ((s_px[row * TW + 32] & 0x00ffffffu) != 0)
				lit++;
		}
		pr_info("gl_test: case 56: %2d lines in one draw -> %2d rows lit\n", n, lit);
		/* One numbered check per line count (checks 1..5 for 2, 4, 8, 16,
		 * 20), got = the rows lit, want = the lines drawn: a board with no
		 * console reads the failing count and how far it got from its CASE
		 * record, where a bare `ok = false` left it NO_CHECK. */
		ok &= gl_test_check(lit == n, (uint32_t)lit, (uint32_t)n);
	}
	return ok ? 0 : -1;
}

static int case_prims(void)
{
	static struct cuv_vertex v[8];
	GLuint prog, ibo;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);

	/* frame 1: a full-screen TRIANGLE_FAN from client-side arrays (no VBO),
	 * magenta, with the colour given as normalised unsigned bytes */
	{
		static const float pos[4][2] = { { -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, 1 } };
		static uint8_t col[4][4];

		for (int i = 0; i < 4; i++) {
			col[i][0] = 255; col[i][1] = 0; col[i][2] = 255; col[i][3] = 255;
			cuv_set(&v[i], pos[i][0], pos[i][1], 0.0f, 1, 0, 1);
		}
		glBindBuffer(GL_ARRAY_BUFFER, 0);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), v[0].pos);
		glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, GL_TRUE, 4, col);
		glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), v[0].uv);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		glEnableVertexAttribArray(2);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLE_FAN, 0, 4);
		glFinish();
		if (!check_gl_error("fan"))
			return -1;
		read_topdown();
		ok &= expect_px("fan: centre", 32, 32, 0xffff00ff, 0);
		ok &= expect_px("fan: corner", 2, 61, 0xffff00ff, 0);
		ok &= expect_px("fan: corner", 61, 2, 0xffff00ff, 0);

		/* the same quad through glDrawElements with u8 indices in a VBO,
		 * then u32 client-side indices, drawing from vertex 1 (first = 1) */
		{
			static const uint8_t idx8[6] = { 0, 1, 2, 0, 2, 3 };
			static const uint32_t idx32[6] = { 0, 1, 2, 0, 2, 3 };

			for (int i = 0; i < 4; i++) { col[i][0] = 0; col[i][1] = 255; col[i][2] = 255; }
			glGenBuffers(1, &ibo);
			glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
			glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx8), idx8, GL_STATIC_DRAW);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, (const void *)0);
			glFinish();
			if (!check_gl_error("u8 elements"))
				return -1;
			read_topdown();
			ok &= expect_px("u8 indices in a VBO", 32, 32, 0xffffff00, 0);

			glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
			for (int i = 0; i < 4; i++) { col[i][0] = 255; col[i][1] = 255; col[i][2] = 0; }
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_INT, idx32);
			glFinish();
			if (!check_gl_error("u32 elements"))
				return -1;
			read_topdown();
			ok &= expect_px("u32 client indices", 32, 32, 0xff00ffff, 0);
		}

		/* normalised unsigned bytes inside a VBO (converted to float at
		 * draw time, as the client-side path) */
		{
			GLuint cbo;

			for (int i = 0; i < 4; i++) { col[i][0] = 0; col[i][1] = 0; col[i][2] = 255; }
			glGenBuffers(1, &cbo);
			glBindBuffer(GL_ARRAY_BUFFER, cbo);
			glBufferData(GL_ARRAY_BUFFER, sizeof(col), col, GL_STATIC_DRAW);
			glVertexAttribPointer(1, 4, GL_UNSIGNED_BYTE, GL_TRUE, 4, (const void *)0);
			glBindBuffer(GL_ARRAY_BUFFER, 0);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLE_FAN, 0, 4);
			glFinish();
			if (!check_gl_error("ubyte vbo"))
				return -1;
			read_topdown();
			ok &= expect_px("ubyte colours in a VBO", 32, 32, 0xffff0000, 0);
		}
	}

	/* the remaining frames use float colours from the struct array */
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), v[0].color);

	/* frame 2: LINE_LOOP square (pixel columns 16 and 48, rows 16 and 48),
	 * 3 px wide, green */
	{
		cuv_set(&v[0], ndc_x(16), ndc_y(16), 0.0f, 0, 1, 0);
		cuv_set(&v[1], ndc_x(48), ndc_y(16), 0.0f, 0, 1, 0);
		cuv_set(&v[2], ndc_x(48), ndc_y(48), 0.0f, 0, 1, 0);
		cuv_set(&v[3], ndc_x(16), ndc_y(48), 0.0f, 0, 1, 0);
		glLineWidth(3.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_LINE_LOOP, 0, 4);
		glFinish();
		glLineWidth(1.0f);
		if (!check_gl_error("line loop"))
			return -1;
		read_topdown();
		dump_map(s_px, TW, TH);
		ok &= expect_px("loop: left edge", 16, 32, 0xff00ff00, 0);
		ok &= expect_px("loop: top edge", 32, 16, 0xff00ff00, 0);
		ok &= expect_px("loop: right edge", 48, 32, 0xff00ff00, 0);
		ok &= expect_px("loop: bottom edge (closing)", 32, 48, 0xff00ff00, 0);
		ok &= expect_px("loop: 3 px wide", 17, 32, 0xff00ff00, 0);
		ok &= expect_px("loop: inside empty", 32, 32, 0xff000000, 0);
		ok &= expect_px("loop: outside empty", 4, 4, 0xff000000, 0);
	}

	/* frame 3: LINES, 1 px: a horizontal line on row 40 from x 8 to 56 and a
	 * vertical one on column 20 from row 8 to 24 */
	{
		cuv_set(&v[0], ndc_x(8), ndc_y(40), 0.0f, 1, 1, 1);
		cuv_set(&v[1], ndc_x(56), ndc_y(40), 0.0f, 1, 1, 1);
		cuv_set(&v[2], ndc_x(20), ndc_y(8), 0.0f, 1, 1, 1);
		cuv_set(&v[3], ndc_x(20), ndc_y(24), 0.0f, 1, 1, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_LINES, 0, 4);
		glFinish();
		if (!check_gl_error("lines"))
			return -1;
		read_topdown();
		ok &= expect_px("lines: on the horizontal", 32, 40, 0xffffffff, 0);
		ok &= expect_px("lines: row above empty", 32, 38, 0xff000000, 0);
		ok &= expect_px("lines: row below empty", 32, 42, 0xff000000, 0);
		ok &= expect_px("lines: beyond the end", 60, 40, 0xff000000, 0);
		ok &= expect_px("lines: on the vertical", 20, 16, 0xffffffff, 0);
		ok &= expect_px("lines: column beside empty", 22, 16, 0xff000000, 0);
	}

	/* frame 4: LINE_STRIP of three points (an L) */
	{
		cuv_set(&v[0], ndc_x(8), ndc_y(8), 0.0f, 1, 0, 0);
		cuv_set(&v[1], ndc_x(8), ndc_y(56), 0.0f, 1, 0, 0);
		cuv_set(&v[2], ndc_x(56), ndc_y(56), 0.0f, 1, 0, 0);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_LINE_STRIP, 0, 3);
		glFinish();
		if (!check_gl_error("line strip"))
			return -1;
		read_topdown();
		ok &= expect_px("strip: first segment", 8, 32, 0xff0000ff, 0);
		ok &= expect_px("strip: second segment", 32, 56, 0xff0000ff, 0);
		ok &= expect_px("strip: not closed", 32, 8, 0xff000000, 0);
	}

	/* frame 5: POINTS with points.vert (gl_PointSize from an attribute):
	 * sizes 1, 3 and 5 px at pixel centres -> square footprints */
	{
		struct pv { float pos[4]; float color[4]; float size; } pv[3];
		GLuint prog2 = link_program(gl_test_points_vs_src, gl_test_interp_fs_src);
		static const uint32_t px[3][2] = { { 8, 8 }, { 40, 24 }, { 56, 56 } };
		static const float sizes[3] = { 1.0f, 3.0f, 5.0f };

		if (!prog2)
			return -1;
		glUseProgram(prog2);
		for (int i = 0; i < 3; i++) {
			pv[i].pos[0] = ndc_x(px[i][0]); pv[i].pos[1] = ndc_y(px[i][1]);
			pv[i].pos[2] = 0.0f; pv[i].pos[3] = 1.0f;
			pv[i].color[0] = 0; pv[i].color[1] = 0; pv[i].color[2] = 1; pv[i].color[3] = 1;
			pv[i].size = sizes[i];
		}
		glBindBuffer(GL_ARRAY_BUFFER, 0);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv[0].pos);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv[0].color);
		glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, sizeof(struct pv), &pv[0].size);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_POINTS, 0, 3);
		glFinish();
		if (!check_gl_error("points"))
			return -1;
		read_topdown();
		for (uint32_t y = 20; y < 29; y++) {
			char row[16];

			for (uint32_t x = 36; x < 45; x++)
				row[x - 36] = s_px[y * TW + x] & 0xffffff ? '#' : '.';
			row[9] = 0;
			pr_info("gl_test:   3 px point at (40,24): %s\n", row);
		}
		ok &= expect_px("1 px point: centre", 8, 8, 0xffff0000, 0);
		ok &= expect_px("1 px point: neighbour empty", 9, 8, 0xff000000, 0);
		ok &= expect_px("1 px point: neighbour empty", 8, 9, 0xff000000, 0);
		ok &= expect_px("3 px point: centre", 40, 24, 0xffff0000, 0);
		ok &= expect_px("3 px point: corner", 41, 25, 0xffff0000, 0);
		ok &= expect_px("3 px point: corner", 39, 23, 0xffff0000, 0);
		ok &= expect_px("3 px point: outside", 42, 24, 0xff000000, 0);
		ok &= expect_px("3 px point: outside", 40, 22, 0xff000000, 0);
		ok &= expect_px("5 px point: edge", 58, 56, 0xffff0000, 0);
		ok &= expect_px("5 px point: outside", 59, 56, 0xff000000, 0);
	}
	return ok ? 0 : -1;
}

/* ---- case 11: colour survives across kicks -------------------------------- */

static int case_persist(void)
{
	static struct cuv_vertex v[3 * 300];   /* 36 KB: never on the kernel stack */
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);

	/* 300 tiny triangles, one per 2x2 block in a 20x15 grid (more than the
	 * backend's draws-per-frame limit, so the frame is split into kicks) */
	for (int i = 0; i < 300; i++) {
		float cx = 4.0f + (float)(i % 20) * 3.0f, cy = 4.0f + (float)(i / 20) * 4.0f;

		cuv_set(&v[i * 3 + 0], ndc_x((uint32_t)cx) - 1.0f / TW, ndc_y((uint32_t)cy) - 1.0f / TH, 0, 1, 1, 0);
		cuv_set(&v[i * 3 + 1], ndc_x((uint32_t)cx) + 3.0f / TW, ndc_y((uint32_t)cy) - 1.0f / TH, 0, 1, 1, 0);
		cuv_set(&v[i * 3 + 2], ndc_x((uint32_t)cx) - 1.0f / TW, ndc_y((uint32_t)cy) + 3.0f / TH, 0, 1, 1, 0);
	}
	upload_vbo(v, sizeof(v));
	cuv_attribs();

	/* frame 1: clear + the slice-A triangle; frame 2: no clear, one small
	 * yellow triangle -> both must be there */
	glClear(GL_COLOR_BUFFER_BIT);
	{
		struct cuv_vertex tri[3];
		GLuint vbo2;

		slice_a_vertices(tri, 0);
		vbo2 = upload_vbo(tri, sizeof(tri));
		cuv_attribs();
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		glDeleteBuffers(1, &vbo2);
	}
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glDrawArrays(GL_TRIANGLES, 0, 3);          /* block (4,4) */
	glFinish();
	if (!check_gl_error("no-clear frame"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("earlier frame kept", 32, 40, A_CENTROID, 3);
	ok &= expect_px("earlier frame kept", 31, 12, A_V0, 3);
	ok &= expect_px("new draw", 4, 4, 0xff00ffff, 0);
	ok &= expect_px("still clear elsewhere", 60, 4, 0xff000000, 0);

	/* frame 2b: a fresh (finished) frame, no clear, 113 draws in one kick */
	glDrawArrays(GL_TRIANGLES, 0, 3);
	for (int i = 1; i < 113; i++)
		glDrawArrays(GL_TRIANGLES, i * 3, 3);
	glFinish();
	if (!check_gl_error("113 draws, no clear"))
		return -1;
	read_topdown();
	ok &= expect_px("no-clear 113: draw 112", 4 + 12 * 3, 4 + 5 * 4, 0xff00ffff, 0);
	ok &= expect_px("no-clear 113: earlier kept", 32, 40, A_CENTROID, 3);

	/* frame 3: clear then 300 draws in one frame -> split kicks keep all */
	glClear(GL_COLOR_BUFFER_BIT);
	for (int i = 0; i < 300; i++)
		glDrawArrays(GL_TRIANGLES, i * 3, 3);
	glFinish();
	if (!check_gl_error("300 draws"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("draw 0", 4, 4, 0xff00ffff, 0);
	ok &= expect_px("draw 255 (last of kick 1)", 4 + 15 * 3, 4 + 12 * 4, 0xff00ffff, 0);
	ok &= expect_px("draw 256 (first of kick 2)", 4 + 16 * 3, 4 + 12 * 4, 0xff00ffff, 0);
	ok &= expect_px("draw 299", 4 + 19 * 3, 4 + 14 * 4, 0xff00ffff, 0);
	ok &= expect_px("triangle from frame 1 cleared", 32, 40, 0xff000000, 0);
	return ok ? 0 : -1;
}

/* ---- case 34: per-draw uniforms within one kick (regression) ------------- */

/* Two triangles drawn with the SAME program but DIFFERENT u_color, both in one
 * frame with no flush between them, so they accumulate into one kick. Each draw
 * must keep the uniform it was recorded with: the left triangle orange, the
 * right one cyan.
 *
 * The uniform block is one buffer per program (gl_draw.c PG(program)). The
 * backend now snapshots each draw's uniform bytes into the frame arena at
 * record time (record_draw, DOUTD/FS-kick blocks in pvr_backend.c), so the
 * second glUniform does not reach back into the first draw. Before that fix
 * both draws pointed at the shared buffer and took the last-written colour, so
 * both triangles came out cyan. That is the exact bug the Slint/FemtoVG
 * dashboard hit: one shader reused for many draws, one colour per draw. */
static int case_unishare(void)
{
	static const float pos[6][2] = {
		{ -0.5f,  0.5f }, { -0.9f, -0.5f }, { -0.1f, -0.5f },   /* left  */
		{  0.5f,  0.5f }, {  0.1f, -0.5f }, {  0.9f, -0.5f },   /* right */
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint loc;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	loc = glGetUniformLocation(prog, "u_color");
	if (loc < 0) {
		pr_err("gl_test: u_color location not found\n");
		return -1;
	}

	/* one frame, two draws, no flush between: they share a kick */
	glClear(GL_COLOR_BUFFER_BIT);
	glUniform4f(loc, 1.0f, 0.5f, 0.0f, 1.0f);   /* orange */
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glUniform4f(loc, 0.0f, 1.0f, 1.0f, 1.0f);   /* cyan */
	glDrawArrays(GL_TRIANGLES, 3, 3);
	glFinish();
	if (!check_gl_error("two same-program draws, one kick"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	/* the bug rendered BOTH triangles cyan (the last uniform wins) */
	ok &= expect_px("draw 1 keeps orange", 16, 40, 0xff0080ff, 2);
	ok &= expect_px("draw 2 is cyan", 48, 40, 0xffffff00, 2);
	return ok ? 0 : -1;
}

/* ---- runner ------------------------------------------------------------ */

/* ---- case 21: RGB565 window surface -------------------------------------- */

/* A 16-bit R5G6B5 window (the SafeUI application surface, DRM RGB565): the
 * PBE packs the FS output into 5/6/5 bits, glReadPixels expands them back,
 * and a frame without glClear keeps the previous one through the load quad
 * (the TPU reads the surface as an R5G6B5 texture). The raw memory is
 * checked too: red must be in the high bits. */
static int case_rgb565(void)
{
	static const EGLint cfg_attribs[] = {
		EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_RED_SIZE, 5, EGL_GREEN_SIZE, 6, EGL_BLUE_SIZE, 5, EGL_ALPHA_SIZE, 0, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window win;
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfgs[4], cfg = NULL, other = NULL;
	EGLSurface ws;
	EGLContext ctx;
	EGLint n = 0, v = 0;
	struct osal_gpu_mem wmem;
	struct cuv_vertex tri[3];
	GLuint prog;
	uint16_t raw = 0;
	bool ok = true;

	release_harness_context();
	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, cfgs, 4, &n) || n < 1)
		return -1;
	for (EGLint i = 0; i < n; i++) {
		eglGetConfigAttrib(dpy, cfgs[i], EGL_BUFFER_SIZE, &v);
		if (v == 16)
			cfg = cfgs[i];
		else
			other = cfgs[i];
	}
	ok &= expect_int("a 16-bit config is offered", cfg != NULL, 1);
	ok &= expect_int("32-bit configs are offered too", other != NULL, 1);
	if (!ok)
		return -1;
	eglGetConfigAttrib(dpy, cfg, EGL_GREEN_SIZE, &v);
	ok &= expect_int("green size 6", v, 6);

	/* the window: a TW x TH RGB565 buffer from the pool (a scan-out buffer
	 * in an application) */
	if (osal_gpu_alloc(yakogl_heap_base(GLES_HEAP_DATA), TW * TH * 2u, OSAL_MEM_DATA, &wmem) != 0)
		return -1;
	win.phys = wmem.phys;
	win.width = TW;
	win.height = TH;
	win.stride_bytes = TW * 2u;
	win.format = GLES_NATIVE_FORMAT_RGB565;
	ok &= expect_int("32-bit config refuses the window",
			 eglCreateWindowSurface(dpy, other, &win, NULL) == EGL_NO_SURFACE &&
			 eglGetError() == EGL_BAD_MATCH, 1);
	ws = eglCreateWindowSurface(dpy, cfg, &win, NULL);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	ok &= expect_int("RGB565 window surface", ws != EGL_NO_SURFACE, 1);
	if (!ok || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, ws, ws, ctx))
		return -1;

	/* frame 1: red background, the slice-A triangle */
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	slice_a_vertices(tri, 0);
	upload_vbo(tri, sizeof(tri));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	ok &= expect_int("eglSwapBuffers", eglSwapBuffers(dpy, ws), EGL_TRUE);
	if (!check_gl_error("draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("565: centroid (5/6-bit steps)", 32, 40, A_CENTROID, 8);
	ok &= expect_px("565: red background", 1, 1, 0xff0000ff, 0);
	/* raw memory: top-down row 1, pixel 1 = 0xF800 (R5 in bits 15..11) */
	if (wmem.cpu) {
		if (g_osal->cache_invd)
			g_osal->cache_invd(wmem.cpu, TW * TH * 2u);
		raw = ((const uint16_t *)wmem.cpu)[1 * TW + 1];
	} else if (g_osal->gpu_read) {
		g_osal->gpu_read(&wmem, (1 * TW + 1) * 2u, &raw, sizeof(raw));
	}
	ok &= expect_int("565: raw red pixel = 0xF800", raw, 0xF800);

	/* frame 2: no clear, a blue triangle in the bottom-left 12x12 corner
	 * (a 12x12 viewport, the uniform-colour program); the rest of the
	 * frame is loaded back from the RGB565 surface */
	{
		GLuint blue = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);

		if (!blue)
			return -1;
		glUseProgram(blue);
		glUniform4f(glGetUniformLocation(blue, "u_color"), 0.0f, 0.0f, 1.0f, 1.0f);
	}
	glViewport(0, 0, 12, 12);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	ok &= expect_int("eglSwapBuffers 2", eglSwapBuffers(dpy, ws), EGL_TRUE);
	if (!check_gl_error("frame 2"))
		return -1;
	read_topdown();
	ok &= expect_px("565: earlier frame kept", 32, 40, A_CENTROID, 8);
	ok &= expect_px("565: background kept", 1, 1, 0xff0000ff, 0);
	ok &= expect_px("565: corner triangle blue", 6, TH - 4, 0xffff0000, 0);

	ok &= expect_int("release", eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT), EGL_TRUE);
	ok &= expect_int("eglDestroySurface", eglDestroySurface(dpy, ws), EGL_TRUE);
	ok &= expect_int("eglDestroyContext", eglDestroyContext(dpy, ctx), EGL_TRUE);
	ok &= expect_int("eglTerminate", eglTerminate(dpy), EGL_TRUE);
	/* the window's buffer goes back to the pool, as case_viewwin's does (the
	 * owner report counted it as unaccounted: 8 KiB at every close) */
	if (g_osal->gpu_free)
		g_osal->gpu_free(&wmem);
	return ok ? 0 : -1;
}

/* ---- case 171: a window that is a view into a wider buffer (viewwin) ------ */

/* A TW x TH window whose rows are VIEWWIN_PITCH pixels apart: a view into a
 * wider scan-out buffer (a window smaller than the panel, shown in place,
 * CONFIG_YAKOGL_BOARD_SCANOUT_VIEW). The whole buffer starts as
 * VIEWWIN_PAD. Frame 1 clears red and draws the slice-A triangle; frame 2
 * draws a blue corner without a clear, so the rest comes back through the
 * colour load at the view's pitch. glReadPixels must read the window's
 * pixels (the first and the last row, the corners), and the raw memory must
 * hold the window's red at the view's pitch and keep the padding after each
 * row: a GPU that wrote at tex_pitch(TW) would put row 1 into row 0's
 * padding and leave the last rows unwritten. All three GPUs. */
#define VIEWWIN_PITCH (TW + 32u) /* 96 pixels: even, and 384 bytes, 64-byte aligned */
#define VIEWWIN_PAD   0x5a5a5a5au

/* The word a pixel of colour `rgba` (glReadPixels' R,G,B,A word) has in the
 * window's memory. The window is RGBA8888, but a pixel engine without that
 * render format (the GC7000: A8R8G8B8) stores it B,G,R,A, which
 * rt_rgba8888_is_bgra says; the padding is raw fill and never goes through
 * here. */
static uint32_t viewwin_word(uint32_t rgba)
{
	if (!g_backend->rt_rgba8888_is_bgra)
		return rgba;
	return (rgba & 0xff00ff00u) | ((rgba & 0xffu) << 16) | ((rgba >> 16) & 0xffu);
}

static uint32_t viewwin_raw(struct osal_gpu_mem *m, uint32_t x, uint32_t row)
{
	uint32_t v = 0;
	size_t off = ((size_t)row * VIEWWIN_PITCH + x) * 4u;

	if (m->cpu) {
		if (g_osal->cache_invd)
			g_osal->cache_invd(m->cpu, (size_t)VIEWWIN_PITCH * TH * 4u);
		v = ((const uint32_t *)m->cpu)[off / 4u];
	} else if (g_osal->gpu_read) {
		g_osal->gpu_read(m, off, &v, sizeof(v));
	}
	return v;
}

static int case_viewwin(void)
{
	static const EGLint cfg_attribs[] = {
		EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window win;
	static uint32_t fill[VIEWWIN_PITCH * TH];
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLSurface ws;
	EGLContext ctx;
	EGLint n = 0;
	struct osal_gpu_mem wmem;
	struct cuv_vertex tri[3];
	GLuint prog, blue;
	bool ok = true;

	release_harness_context();
	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n < 1)
		return -1;
	if (osal_gpu_alloc(yakogl_heap_base(GLES_HEAP_DATA), VIEWWIN_PITCH * TH * 4u, OSAL_MEM_DATA, &wmem) != 0)
		return -1;
	for (uint32_t i = 0; i < VIEWWIN_PITCH * TH; i++)
		fill[i] = VIEWWIN_PAD;
	if (wmem.cpu) {
		memcpy(wmem.cpu, fill, sizeof(fill));
		if (g_osal->cache_flush)
			g_osal->cache_flush(wmem.cpu, sizeof(fill));
	} else if (g_osal->gpu_write) {
		g_osal->gpu_write(&wmem, 0, fill, sizeof(fill));
	}
	win.phys = wmem.phys;
	win.width = TW;
	win.height = TH;
	win.stride_bytes = VIEWWIN_PITCH * 4u;
	win.format = GLES_NATIVE_FORMAT_RGBA8888;
	ws = eglCreateWindowSurface(dpy, cfg, &win, NULL);
	ok &= expect_int("a view window (stride wider than the width)", ws != EGL_NO_SURFACE, 1);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (!ok || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, ws, ws, ctx))
		return -1;

	/* frame 1: red background, the slice-A triangle */
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	slice_a_vertices(tri, 0);
	upload_vbo(tri, sizeof(tri));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	ok &= expect_int("eglSwapBuffers", eglSwapBuffers(dpy, ws), EGL_TRUE);
	if (!check_gl_error("draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("view: centroid", 32, 40, A_CENTROID, 3);
	ok &= expect_px("view: first row, left", 0, 0, 0xff0000ff, 0);
	ok &= expect_px("view: first row, right", TW - 1, 0, 0xff0000ff, 0);
	ok &= expect_px("view: last row, left", 0, TH - 1, 0xff0000ff, 0);
	ok &= expect_px("view: last row, right", TW - 1, TH - 1, 0xff0000ff, 0);

	/* raw memory: the window's red at the view's pitch, the padding kept
	 * (top-down rows: memory row 0 is the top) */
	ok &= expect_int("raw: row 0, last pixel red", viewwin_raw(&wmem, TW - 1, 0), viewwin_word(0xff0000ff));
	ok &= expect_int("raw: row 0, first padding pixel kept", viewwin_raw(&wmem, TW, 0), VIEWWIN_PAD);
	ok &= expect_int("raw: row 0, last padding pixel kept", viewwin_raw(&wmem, VIEWWIN_PITCH - 1, 0), VIEWWIN_PAD);
	ok &= expect_int("raw: last row, first pixel red", viewwin_raw(&wmem, 0, TH - 1), viewwin_word(0xff0000ff));
	ok &= expect_int("raw: last row, first padding pixel kept", viewwin_raw(&wmem, TW, TH - 1), VIEWWIN_PAD);

	/* frame 2: no clear, a blue triangle in the bottom-left 12x12 corner;
	 * the rest is loaded back from the view at its pitch */
	blue = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!blue)
		return -1;
	glUseProgram(blue);
	glUniform4f(glGetUniformLocation(blue, "u_color"), 0.0f, 0.0f, 1.0f, 1.0f);
	glViewport(0, 0, 12, 12);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	ok &= expect_int("eglSwapBuffers 2", eglSwapBuffers(dpy, ws), EGL_TRUE);
	if (!check_gl_error("frame 2"))
		return -1;
	read_topdown();
	ok &= expect_px("view: earlier frame kept", 32, 40, A_CENTROID, 3);
	ok &= expect_px("view: first row kept", TW - 1, 0, 0xff0000ff, 0);
	ok &= expect_px("view: corner triangle blue", 6, TH - 4, 0xffff0000, 0);
	ok &= expect_int("raw: padding still kept", viewwin_raw(&wmem, TW + 1, TH - 1), VIEWWIN_PAD);

	ok &= expect_int("release", eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT), EGL_TRUE);
	ok &= expect_int("eglDestroySurface", eglDestroySurface(dpy, ws), EGL_TRUE);
	ok &= expect_int("eglDestroyContext", eglDestroyContext(dpy, ctx), EGL_TRUE);
	ok &= expect_int("eglTerminate", eglTerminate(dpy), EGL_TRUE);
	if (g_osal->gpu_free)
		g_osal->gpu_free(&wmem);
	return ok ? 0 : -1;
}

/* ---- case 22: vertex array objects (GL_OES_vertex_array_object) ------------ */

/* A VAO records the vertex-attribute arrays and the element-buffer binding. The
 * slice-A triangle is set up inside a VAO; binding the default VAO (0) drops
 * that state (no enabled arrays -> a degenerate draw, nothing rendered); binding
 * the VAO again restores it and the triangle is back. Also core ES 3.0 names. */
static int case_vao(void)
{
	struct cuv_vertex tri[3];
	GLuint prog, vao = 0;
	GLint iv = 0;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	slice_a_vertices(tri, 0);

	ok &= expect_int("GL_OES_vertex_array_object advertised",
			 strstr((const char *)glGetString(GL_EXTENSIONS),
				"GL_OES_vertex_array_object") != NULL, 1);
	glGenVertexArraysOES(1, &vao);
	ok &= expect_int("glGenVertexArrays returns a nonzero name", vao != 0, 1);
	/* ES 3.0 6.1.10 and OES_vertex_array_object: glGenVertexArrays only
	 * reserves the name. The object comes into being at its first bind, so
	 * glIsVertexArray answers false until then -- which is what
	 * dEQP-GLES3.functional.negative_api.state.is_vertex_array requires in
	 * as many words. This check used to want true and had been failing on
	 * every backend since the library was made to follow the rule. */
	ok &= expect_int("generated name is not a VAO yet", glIsVertexArrayOES(vao), 0);

	/* record the triangle's array state INTO the VAO */
	glBindVertexArrayOES(vao);
	ok &= expect_int("bound name is a VAO", glIsVertexArrayOES(vao), 1);
	glGetIntegerv(0x85B5 /* GL_VERTEX_ARRAY_BINDING */, &iv);
	ok &= expect_int("VERTEX_ARRAY_BINDING == vao", iv, (long)vao);
	upload_vbo(tri, sizeof(tri));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("vao draw"))
		return -1;
	read_topdown();
	ok &= expect_px("vao: centroid", 32, 40, A_CENTROID, 8);

	/* default VAO (0): its arrays were never enabled -> nothing renders */
	glBindVertexArrayOES(0);
	iv = 1;
	glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_ENABLED, &iv);
	ok &= expect_int("default VAO attr0 disabled", iv, 0);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	read_topdown();
	ok &= expect_px("default VAO: centroid empty", 32, 40, 0xff000000, 0);

	/* rebinding restores the recorded state: the triangle is back */
	glBindVertexArrayOES(vao);
	iv = 0;
	glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_ENABLED, &iv);
	ok &= expect_int("VAO restored attr0 enabled", iv, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	read_topdown();
	ok &= expect_px("vao rebound: centroid", 32, 40, A_CENTROID, 8);

	glBindVertexArrayOES(0);
	glDeleteVertexArraysOES(1, &vao);
	ok &= expect_int("deleted name is not a VAO", glIsVertexArrayOES(vao), 0);
	return ok ? 0 : -1;
}

/* ---- case 23: nonzero-winding fill with a hole (FemtoVG path-fill) --------- */

/* FemtoVG fills a path by stencil-then-cover with the nonzero winding rule: a
 * stencil pass with no colour write and two-sided INCR_WRAP(front)/
 * DECR_WRAP(back) accumulates the winding number, then a cover pass with
 * stencil != 0 fills. Here an outer CCW triangle (+1) contains an inner CW
 * triangle (-1); the inner region nets 0 (a hole) and is left empty. */
static int case_winding(void)
{
	struct cuv_vertex v[6];
	GLuint prog;
	bool ok = true;

	/* outer CCW triangle (front-facing) -> INCR_WRAP -> stencil 1 */
	cuv_set(&v[0], -0.8f, -0.7f, 0.0f, 1, 1, 0);
	cuv_set(&v[1],  0.8f, -0.7f, 0.0f, 1, 1, 0);
	cuv_set(&v[2],  0.0f,  0.8f, 0.0f, 1, 1, 0);
	/* inner triangle wound CW (back-facing) -> DECR_WRAP -> back to 0: a hole */
	cuv_set(&v[3], -0.25f, -0.10f, 0.0f, 1, 1, 0);
	cuv_set(&v[4],  0.00f,  0.35f, 0.0f, 1, 1, 0);
	cuv_set(&v[5],  0.25f, -0.10f, 0.0f, 1, 1, 0);

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glFrontFace(GL_CCW);
	glDisable(GL_CULL_FACE);

	/* stencil pass: winding into the stencil buffer, no colour */
	glEnable(GL_STENCIL_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClearStencil(0);
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glStencilFunc(GL_ALWAYS, 0, 0xff);
	glStencilOpSeparate(GL_FRONT, GL_KEEP, GL_KEEP, GL_INCR_WRAP);
	glStencilOpSeparate(GL_BACK,  GL_KEEP, GL_KEEP, GL_DECR_WRAP);
	glDrawArrays(GL_TRIANGLES, 0, 6);

	/* cover pass: fill winding != 0 (the same geometry bounds the filled area) */
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glStencilFunc(GL_NOTEQUAL, 0, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	glDisable(GL_STENCIL_TEST);
	if (!check_gl_error("winding"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("winding: outer filled", 19, 42, 0xff00ffff, 0);
	ok &= expect_px("winding: hole empty",   32, 30, 0xff000000, 0);
	ok &= expect_px("winding: outside empty", 4,  4, 0xff000000, 0);
	return ok ? 0 : -1;
}

/* ---- case 24: sampler compaction (a used non-zero binding lands at sh0) ---- */

/* gl_test_compact_fs_src declares two samplers but samples only binding 1, so
 * the compiler drops binding 0 and compacts binding 1 to shared register 0.
 * The backend must place the used sampler's descriptor there: bind red to
 * unit 0 (unused) and green to unit 1 (u_tex1) and the draw must read green. */
static int case_compact(void)
{
	static const uint8_t red[4]   = { 255, 0, 0, 255 };
	static const uint8_t green[4] = { 0, 255, 0, 255 };
	GLuint prog, t0 = 0, t1 = 0;
	GLint l0, l1;
	bool ok = true;

	prog = link_program(gl_test_uv_vs_src, gl_test_compact_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();

	glActiveTexture(GL_TEXTURE0);
	glGenTextures(1, &t0);
	glBindTexture(GL_TEXTURE_2D, t0);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, red);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);

	glActiveTexture(GL_TEXTURE1);
	glGenTextures(1, &t1);
	glBindTexture(GL_TEXTURE_2D, t1);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, green);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glActiveTexture(GL_TEXTURE0);

	l0 = glGetUniformLocation(prog, "u_tex0");
	l1 = glGetUniformLocation(prog, "u_tex1");
	ok &= expect_int("unused sampler u_tex0 optimized out (loc -1)", l0, -1);
	ok &= expect_int("used sampler u_tex1 has a location", l1 >= 0, 1);
	glUniform1i(l0, 0);   /* -1: a no-op, as on a conformant driver */
	glUniform1i(l1, 1);

	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("compact draw"))
		return -1;
	read_topdown();
	ok &= expect_px("compacted sampler reads unit 1 (green)", 32, 32, 0xff00ff00, 0);
	glDeleteTextures(1, &t0);
	glDeleteTextures(1, &t1);
	return ok ? 0 : -1;
}

/* ---- case 25: continuous-render stress (reproduce the VDM/tiler HWR stall) -- */

/* Many multi-draw kicks, like the SafeUI gpu_demo that trips the intermittent
 * VDM/tiler context-store HWR (seen at 366..5000 frames). Not in the default
 * suite; run with GL_TEST_CASE=25 and watch the console for "HWR during fence".
 * Returns non-zero if any kick reports a GL error. */
static int case_stress(void)
{
	GLuint prog;
	struct cuv_vertex v[3 * 8];
	const int kicks = 6000;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);
	{
		GLuint vbo;   /* one reused VBO (do not leak a buffer per kick) */

		glGenBuffers(1, &vbo);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		cuv_attribs();
	}
	for (int k = 0; k < kicks; k++) {
		/* 8 triangles per kick (a fan-ish spread that changes every frame),
		 * a long-ish VDM stream that stresses the tiler/context store */
		for (int t = 0; t < 8; t++) {
			float o = (float)((k + t * 37) % 100) / 100.0f - 0.5f;

			cuv_set(&v[t * 3 + 0], -0.8f + o, -0.8f, 0.0f, 1, 0, 0);
			cuv_set(&v[t * 3 + 1],  0.0f + o,  0.8f, 0.0f, 0, 1, 0);
			cuv_set(&v[t * 3 + 2],  0.8f + o, -0.8f, 0.0f, 0, 0, 1);
		}
		glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STREAM_DRAW);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3 * 8);
		glFinish();
		GLenum e = glGetError();

		if (e != GL_NO_ERROR) {
			pr_err("gl_test: stress: GL error 0x%x at kick %d\n", e, k);
			return -1;
		}
		if ((k % 500) == 0)
			pr_info("gl_test: stress: %d/%d kicks ok\n", k, kicks);
	}
	pr_info("gl_test: stress: %d kicks, no HWR\n", kicks);
	return 0;
}

/* ---- case 26: heavy-geometry stress (long geom phase per kick) ------------- */

/* Case 25 draws only 8 triangles/kick -- a short geom phase that finishes well
 * inside one FW scheduler time-slice, so the VDM never context-stores and the
 * stall never appears (6000 kicks clean). gpu_demo trips the stall because its
 * frames are long enough that the FW preempts the geom context mid-render
 * (VDM_CTX_STORE=1 in the fault signature). This case reproduces that: a very
 * long single geom phase per kick (thousands of primitives), so one kick's TA
 * phase spans a time-slice and forces a mid-render VDM context store. If the
 * store base (CR_VDM_CONTEXT_STATE_BASE) is unset, the tiler freezes -> HWR.
 * Big buffer is global, not on the 16 KiB kernel stack. Watch the console for
 * "HWR during fence". Not in the default suite; run with gl_case=26. */
#define HEAVY_TRIS 2048
static struct cuv_vertex g_heavy[3 * HEAVY_TRIS];

static int case_heavy(void)
{
	GLuint prog, vbo;
	const int kicks = 3000;
	const int reps = 4;   /* draws accumulated into one kick */

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);

	/* Fill once: many small triangles spread across the target so every one
	 * gets binned (real geom work), overlap is fine. */
	for (int t = 0; t < HEAVY_TRIS; t++) {
		float o = (float)(t % 128) / 128.0f - 0.5f;
		float y = (float)((t / 128) % 8) / 8.0f - 0.5f;

		cuv_set(&g_heavy[t * 3 + 0], -0.9f + o, -0.9f + y, 0.0f, 1, 0, 0);
		cuv_set(&g_heavy[t * 3 + 1], -0.9f + o,  0.9f + y, 0.0f, 0, 1, 0);
		cuv_set(&g_heavy[t * 3 + 2],  0.9f + o, -0.9f + y, 0.0f, 0, 0, 1);
	}

	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(g_heavy), g_heavy, GL_STATIC_DRAW);
	cuv_attribs();

	for (int k = 0; k < kicks; k++) {
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		/* reps draws accumulate into one long VDM stream (one kick) */
		for (int r = 0; r < reps; r++)
			glDrawArrays(GL_TRIANGLES, 0, 3 * HEAVY_TRIS);
		glFinish();
		GLenum e = glGetError();

		if (e != GL_NO_ERROR) {
			pr_err("gl_test: heavy: GL error 0x%x at kick %d\n", e, k);
			return -1;
		}
		if ((k % 250) == 0)
			pr_info("gl_test: heavy: %d/%d kicks ok (%d tris/kick)\n",
				k, kicks, reps * HEAVY_TRIS);
	}
	pr_info("gl_test: heavy: %d kicks, no HWR\n", kicks);
	return 0;
}

/* ---- case 27: FemtoVG "invisible fill" repro (position at a non-zero vi_reg) -- */

/* Every working gl_test VS declares its position attribute first, so pco_shc
 * places it at vertex-input register 0. The FemtoVG VS (femtovg/main.vert) is the
 * only shader whose position (`vertex`) does NOT land at vi_reg 0: because
 * `ftcoord = tcoord;` precedes `fpos = vertex;`, pco_shc assigns tcoord -> vi0 and
 * vertex -> vi2. On silicon the FemtoVG fill draws emit no pixels even though every
 * GL-state and command-stream field checks out, so the prime suspect is exactly
 * that non-zero position register. This case isolates it in gl_test's fast
 * environment: it links the existing offline blobs of femtovg/main.vert +
 * femtovg/fill_color.frag (SHA-matched in shader_blobs.c; NOT regenerated here),
 * draws one large triangle in FemtoVG pixel space with a plain-green FillColor, and
 * reads back the centre pixel. Green => the fill rendered; the clear colour => the
 * bug is reproduced in gl_test with no Rust/FemtoVG in the loop. */
static int case_fvgrepro(void)
{
	/* interleaved [x,y (vec2 vertex), u,v (vec2 tcoord)], stride 16 (matches the
	 * FemtoVG vertex layout the VS reads: `vertex` then `tcoord`). */
	struct fvg_vertex { float xy[2]; float uv[2]; };
	/* One big triangle that robustly covers the whole 64x64 target (so the centre
	 * pixel is unambiguously inside, never on an edge). In FemtoVG pixel space the
	 * VS maps pixel -> NDC via viewSize; verts outside [0,TW]x[0,TH] just clip. */
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	/* frag[14] fragment uniform block, laid out per fill_color.frag's #defines.
	 * All zero except: innerCol = frag[6] = green; scissorExt = frag[8].xy = (1,1);
	 * scissorScale = frag[8].zw = (1,1); strokeMult = frag[10].x = 1.0;
	 * strokeThr = frag[10].y = -1.0; shaderType = frag[10].w = 5.0 (FillColor).
	 * scissorMat (frag[0..2]) and scissorRadius (frag[12].w) stay 0 -> scissorMask
	 * clamps to 1, so result = innerCol * strokeAlpha(=1) * scissor(=1) = green. */
	float frag[56] = { 0 };
	GLuint vs, fs, prog, vbo;
	GLint view_loc, frag_loc;
	uint32_t center;
	bool green;

	frag[6 * 4 + 0] = 0.0f; frag[6 * 4 + 1] = 1.0f; frag[6 * 4 + 2] = 0.0f; frag[6 * 4 + 3] = 1.0f; /* innerCol */
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;   /* scissorExt  */
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;   /* scissorScale */
	frag[10 * 4 + 0] = 1.0f;    /* strokeMult */
	frag[10 * 4 + 1] = -1.0f;   /* strokeThr  */
	frag[10 * 4 + 3] = 5.0f;    /* shaderType = SHADER_TYPE_FillColor */

	/* Link with the FemtoVG attribute bindings: vertex -> 0, tcoord -> 1, bound
	 * BEFORE link exactly as FemtoVG's renderer does. link_program() cannot do
	 * this, so build the program inline. */
	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvg_fill_color_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	{
		GLint ok = 0;

		glGetProgramiv(prog, GL_LINK_STATUS, &ok);
		if (!ok) {
			char log[256] = "";

			glGetProgramInfoLog(prog, sizeof(log), NULL, log);
			pr_err("gl_test: fvgrepro: link failed: %s\n", log);
			return -1;
		}
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	frag_loc = glGetUniformLocation(prog, "frag");
	pr_info("gl_test: fvgrepro: viewSize loc=%d, frag loc=%d\n", view_loc, frag_loc);
	glUniform2f(view_loc, (float)TW, (float)TH);
	glUniform4fv(frag_loc, 14, frag);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);   /* distinct non-green clear colour */
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgrepro draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	/* green fill = 0xff00ff00 (AABBGGRR); clear colour ~= 0xff401919. */
	green = chan_diff(center, 0xff00ff00) <= 8;
	pr_info("gl_test: fvgrepro: center=0x%08x EXPECT green(0xff00ff00) -> %s\n",
		center, green ? "GREEN (fill rendered)" :
		"NOT GREEN (invisible-fill bug reproduced)");
	return green ? 0 : -1;
}

/* ---- case 28: isolate the FemtoVG VS -- main.vert + a TRIVIAL solid-green FS
 * that ignores all varyings/uniforms. If this renders green, main.vert produces
 * a correct gl_Position and the fault is in fill_color.frag (coverage); if it is
 * still not green, main.vert itself emits a bad position. */
static int case_fvgvs(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	GLuint vs, fs, prog, vbo;
	GLint view_loc, ok = 0;
	uint32_t center;
	bool green;

	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvgtest_green_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: fvgvs: link failed: %s\n", log);
		return -1;
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	glUniform2f(view_loc, (float)TW, (float)TH);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgvs draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	green = chan_diff(center, 0xff00ff00) <= 8;
	pr_info("gl_test: fvgvs: center=0x%08x EXPECT green(0xff00ff00) -> %s\n",
		center, green ? "GREEN (VS position OK; fill_color.frag is the fault)" :
		"NOT GREEN (main.vert emits a bad position)");
	return green ? 0 : -1;
}

/* ---- case 29: does the ftcoord varying (tcoord at vtxin2 after the hoist)
 * reach the FS correctly? main.vert + an FS that writes vec4(ftcoord,0,1). The
 * VBO sets tcoord=(0.5,1.0) on all 3 verts, so the FS output must be
 * R~0.5(0x80) G~1.0(0xff) B=0 -> pixel ~0xff00ff80. A wrong value means the
 * varying (fed from vtxin2) is not delivered. */
static int case_fvgft(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	GLuint vs, fs, prog, vbo;
	GLint view_loc, ok = 0;
	uint32_t center;
	unsigned r, g, b;
	bool good;

	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvgtest_ftcoord_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: fvgft: link failed: %s\n", log);
		return -1;
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	glUniform2f(view_loc, (float)TW, (float)TH);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgft draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	r = center & 0xff; g = (center >> 8) & 0xff; b = (center >> 16) & 0xff;   /* AABBGGRR */
	good = r > 0x60 && r < 0xa0 && g > 0xe0 && b < 0x20;
	pr_info("gl_test: fvgft: center=0x%08x (R=%u G=%u B=%u) EXPECT ftcoord(0.5,1)->R~128 G~255 B~0 -> %s\n",
		center, r, g, b, good ? "OK (ftcoord delivered)" :
		"WRONG (varying from vtxin2 not delivered)");
	return good ? 0 : -1;
}

/* ---- case 30: does the frag[14] push-constant block reach fill_color's FS?
 * main.vert + an FS that just writes innerCol (=frag[6]). Set frag[6]=green. If
 * green renders, the uniform block IS delivered and fill_color's fault is in its
 * coverage math; if not, the frag[] block read is the fault. */
static int case_fvgin(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	float frag[56] = { 0 };
	GLuint vs, fs, prog, vbo;
	GLint view_loc, frag_loc, ok = 0;
	uint32_t center;
	bool green;

	frag[6 * 4 + 0] = 0.0f; frag[6 * 4 + 1] = 1.0f; frag[6 * 4 + 2] = 0.0f; frag[6 * 4 + 3] = 1.0f;

	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvgtest_inner_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: fvgin: link failed: %s\n", log);
		return -1;
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	frag_loc = glGetUniformLocation(prog, "frag");
	glUniform2f(view_loc, (float)TW, (float)TH);
	glUniform4fv(frag_loc, 14, frag);
	pr_info("gl_test: fvgin: viewSize loc=%d frag loc=%d\n", view_loc, frag_loc);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgin draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	green = chan_diff(center, 0xff00ff00) <= 8;
	pr_info("gl_test: fvgin: center=0x%08x EXPECT green(0xff00ff00) -> %s\n",
		center, green ? "GREEN (frag[] block delivered; fault is coverage math)" :
		"NOT GREEN (frag[] uniform block not delivered)");
	return green ? 0 : -1;
}

/* ---- case 31: show fill_color's coverage terms. Same frag[] setup as case 27,
 * FS writes vec4(strokeMask(), scissorMask(fpos), 0, 1). Expect R~255 (strokeMask
 * =1) and G~255 (scissorMask=1). A low R means strokeMult/ftcoord wrong; a low G
 * means scissorExt/scissorScale/scissorMat wrong. */
static int case_fvgcov(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	float frag[56] = { 0 };
	GLuint vs, fs, prog, vbo;
	GLint view_loc, frag_loc, ok = 0;
	uint32_t center;
	unsigned r, g;

	frag[6 * 4 + 1] = 1.0f; frag[6 * 4 + 3] = 1.0f;  /* innerCol (unused here) */
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;   /* scissorExt  */
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;   /* scissorScale */
	frag[10 * 4 + 0] = 1.0f;    /* strokeMult */
	frag[10 * 4 + 1] = -1.0f;   /* strokeThr  */
	frag[10 * 4 + 3] = 5.0f;    /* shaderType */

	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvgtest_cover_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: fvgcov: link failed: %s\n", log);
		return -1;
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	frag_loc = glGetUniformLocation(prog, "frag");
	glUniform2f(view_loc, (float)TW, (float)TH);
	glUniform4fv(frag_loc, 14, frag);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgcov draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	r = center & 0xff; g = (center >> 8) & 0xff;   /* AABBGGRR: R=strokeMask, G=scissorMask */
	pr_info("gl_test: fvgcov: center=0x%08x strokeMask=R=%u scissorMask=G=%u (want ~255,~255)\n",
		center, r, g);
	return (r > 0xe0 && g > 0xe0) ? 0 : -1;
}

/* ---- case 32: fill_color's full body incl. `discard` (but not its unused
 * samplers/dither). Same frag[] setup as case 27, strokeThr=-1 so discard never
 * fires. GREEN => the discard/texkill USC is fine and fill_color's fault is the
 * dropped samplers or dither; NOT GREEN => the discard itself is the fault. */
static int case_fvgdis(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	float frag[56] = { 0 };
	GLuint vs, fs, prog, vbo;
	GLint view_loc, frag_loc, ok = 0;
	uint32_t center;
	bool green;

	frag[6 * 4 + 1] = 1.0f; frag[6 * 4 + 3] = 1.0f;  /* innerCol = green */
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;
	frag[10 * 4 + 0] = 1.0f;    /* strokeMult */
	frag[10 * 4 + 1] = -1.0f;   /* strokeThr  */
	frag[10 * 4 + 3] = 5.0f;

	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvgtest_discard_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: fvgdis: link failed: %s\n", log);
		return -1;
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	frag_loc = glGetUniformLocation(prog, "frag");
	glUniform2f(view_loc, (float)TW, (float)TH);
	glUniform4fv(frag_loc, 14, frag);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgdis draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	green = chan_diff(center, 0xff00ff00) <= 8;
	pr_info("gl_test: fvgdis: center=0x%08x EXPECT green -> %s\n",
		center, green ? "GREEN (discard USC ok; fault is samplers/dither)" :
		"NOT GREEN (the discard/texkill is the fault)");
	return green ? 0 : -1;
}

/* ---- case 33: a texkill that can NEVER fire (if ftcoord.y < -100 discard),
 * then writes innerCol=green. GREEN => the mere presence of a texkill USC instr
 * is fine (so case 32's fault is strokeThr delivery); NOT GREEN => any texkill
 * shader kills all fragments (a USC/ISP texkill mechanism bug). */
static int case_fvgdis2(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	float frag[56] = { 0 };
	GLuint vs, fs, prog, vbo;
	GLint view_loc, frag_loc, ok = 0;
	uint32_t center;
	bool green;

	frag[6 * 4 + 1] = 1.0f; frag[6 * 4 + 3] = 1.0f;  /* innerCol = green */

	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvgtest_discard2_fs_src);
	if (!vs || !fs)
		return -1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glBindAttribLocation(prog, 0, "vertex");
	glBindAttribLocation(prog, 1, "tcoord");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[256] = "";

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		pr_err("gl_test: fvgdis2: link failed: %s\n", log);
		return -1;
	}
	glUseProgram(prog);

	vbo = upload_vbo(v, sizeof(v));
	(void)vbo;
	glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	view_loc = glGetUniformLocation(prog, "viewSize");
	frag_loc = glGetUniformLocation(prog, "frag");
	glUniform2f(view_loc, (float)TW, (float)TH);
	glUniform4fv(frag_loc, 14, frag);

	glViewport(0, 0, TW, TH);
	glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("fvgdis2 draw"))
		return -1;

	read_topdown();
	dump_map(s_px, TW, TH);
	center = s_px[(TH / 2) * TW + (TW / 2)];
	green = chan_diff(center, 0xff00ff00) <= 8;
	pr_info("gl_test: fvgdis2: center=0x%08x EXPECT green -> %s\n",
		center, green ? "GREEN (never-taken texkill is fine)" :
		"NOT GREEN (any texkill kills all fragments)");
	return green ? 0 : -1;
}

/* Case 35 "tex1d": FemtoVG's multi-stop gradient look-up texture. femtovg
 * allocates a 256x1 RGBA8 image with no data (glTexImage2D NULL), fills it with
 * glTexSubImage2D, samples it with GL_REPEAT on T and linear filtering; a
 * height-1 texture is the one shape no other case covers. The ramp encodes the
 * texel index (R = i, G = 255 - i, B = 128), so a wrong fetch names itself. */
static int case_tex1d(void)
{
	static uint8_t ramp[256 * 4];
	GLuint prog, tex = 0;
	bool ok = true;

	for (uint32_t i = 0; i < 256; i++) {
		ramp[i * 4 + 0] = (uint8_t)i;
		ramp[i * 4 + 1] = (uint8_t)(255 - i);
		ramp[i * 4 + 2] = 128;
		ramp[i * 4 + 3] = 255;
	}
	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 256, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 256, 1, GL_RGBA, GL_UNSIGNED_BYTE, ramp);
	if (!check_gl_error("tex1d upload"))
		return -1;

	for (int pass = 0; pass < 2; pass++) {
		/* pass 0: what femtovg sets (linear, REPEAT_Y); pass 1: nearest + clamp */
		GLint filter = pass == 0 ? GL_LINEAR : GL_NEAREST;
		GLint wrap_t = pass == 0 ? GL_REPEAT : GL_CLAMP_TO_EDGE;

		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap_t);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("tex1d draw"))
			return -1;
		read_topdown();
		pr_info("gl_test: tex1d pass %d (%s): row 32 = %08x %08x %08x %08x %08x %08x %08x %08x\n",
			pass, pass == 0 ? "linear, repeat T" : "nearest, clamp",
			s_px[32 * TW + 4], s_px[32 * TW + 12], s_px[32 * TW + 20], s_px[32 * TW + 28],
			s_px[32 * TW + 36], s_px[32 * TW + 44], s_px[32 * TW + 52], s_px[32 * TW + 60]);
		/* screen x (of 64) samples u = (x + 0.5) / 64 -> texel ~ 4x + 2; the
		 * top and bottom rows read the same texel row (height 1) */
		ok &= expect_px("x=16 top", 16, 4, 0xff80bd42, 12);
		ok &= expect_px("x=16 mid", 16, 32, 0xff80bd42, 12);
		ok &= expect_px("x=16 bottom", 16, 60, 0xff80bd42, 12);
		ok &= expect_px("x=48 mid", 48, 32, 0xff803dc2, 12);
	}
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* Case 36 "fvgconic": FemtoVG's conic gradients on silicon (the one femtovg
 * paint no earlier case exercises; Slint draws the cluster's dial bands with
 * them). Links femtovg main.vert with fill_gradient_conic.frag (SELECT_SHADER 8,
 * two colours mixed by the angle) and fill_image_gradient_conic.frag
 * (SELECT_SHADER 9, the angle indexes a 256x1 look-up texture, case 35's ramp).
 * paintMat translates the centre of the 64x64 target to the origin; the FS
 * measures the angle clockwise from 3 o'clock (y down), so right = 0, below
 * = 1/4, left = 1/2, above = 3/4 of the ramp. */
static int case_fvgconic(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct fvg_vertex v[3] = {
		{ { -(float)TW, -(float)TH }, { 0.5f, 1.0f } },
		{ { 3.0f * TW, -(float)TH }, { 0.5f, 1.0f } },
		{ {      TW,    3.0f * TH }, { 0.5f, 1.0f } },
	};
	static uint8_t ramp[256 * 4];
	bool ok = true;

	for (uint32_t i = 0; i < 256; i++) {
		ramp[i * 4 + 0] = (uint8_t)i;
		ramp[i * 4 + 1] = (uint8_t)(255 - i);
		ramp[i * 4 + 2] = 128;
		ramp[i * 4 + 3] = 255;
	}

	/* passes 2 and 3 repeat 0 and 1 with conicStartAngle = -4 rad: Slint's dial
	 * bands start at -140 degrees plus the renderer's -90 degree correction, so
	 * the angle fraction is computed from a negative argument (fract of it). */
	for (int pass = 0; pass < 4; pass++) {
		float frag[56] = { 0 };
		GLuint vs, fs, prog, vbo, tex = 0;
		bool lut = (pass & 1) != 0;
		GLint view_loc, frag_loc, tex_loc, linked = 0;

		/* paintMat = mat3(frag[3].xyz, frag[4].xyz, frag[5].xyz), column major:
		 * identity with the translation (-cx, -cy) in the third column */
		frag[3 * 4 + 0] = 1.0f;
		frag[4 * 4 + 1] = 1.0f;
		frag[5 * 4 + 0] = -(float)TW / 2.0f;
		frag[5 * 4 + 1] = -(float)TH / 2.0f;
		frag[5 * 4 + 2] = 1.0f;
		frag[6 * 4 + 0] = 1.0f; frag[6 * 4 + 3] = 1.0f;   /* innerCol red  */
		frag[7 * 4 + 2] = 1.0f; frag[7 * 4 + 3] = 1.0f;   /* outerCol blue */
		frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;   /* scissorExt   */
		frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;   /* scissorScale */
		frag[10 * 4 + 0] = 1.0f;    /* strokeMult */
		frag[10 * 4 + 1] = -1.0f;   /* strokeThr  */
		frag[10 * 4 + 3] = lut ? 9.0f : 8.0f;   /* shaderType */
		frag[13 * 4 + 0] = pass < 2 ? 0.0f : -4.0f;    /* conicStartAngle */

		vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
		fs = compile(GL_FRAGMENT_SHADER, lut ? gl_test_fvg_imgconic_fs_src
						       : gl_test_fvg_conic_fs_src);
		if (!vs || !fs)
			return -1;
		prog = glCreateProgram();
		glAttachShader(prog, vs);
		glAttachShader(prog, fs);
		glBindAttribLocation(prog, 0, "vertex");
		glBindAttribLocation(prog, 1, "tcoord");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &linked);
		if (!linked) {
			pr_err("gl_test: fvgconic pass %d: link failed\n", pass);
			return -1;
		}
		glUseProgram(prog);

		vbo = upload_vbo(v, sizeof(v));
		(void)vbo;
		glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);

		if (lut) {
			glGenTextures(1, &tex);
			glActiveTexture(GL_TEXTURE0);
			glBindTexture(GL_TEXTURE_2D, tex);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 256, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
			glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 256, 1, GL_RGBA, GL_UNSIGNED_BYTE, ramp);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
			tex_loc = glGetUniformLocation(prog, "tex");
			if (tex_loc >= 0)
				glUniform1i(tex_loc, 0);
		}
		view_loc = glGetUniformLocation(prog, "viewSize");
		frag_loc = glGetUniformLocation(prog, "frag");
		glUniform2f(view_loc, (float)TW, (float)TH);
		glUniform4fv(frag_loc, 14, frag);

		glViewport(0, 0, TW, TH);
		glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 3);
		glFinish();
		if (!check_gl_error("fvgconic draw"))
			return -1;

		read_topdown();
		pr_info("gl_test: fvgconic pass %d (%s, start angle %s):\n", pass,
			lut ? "256x1 look-up ramp" : "two colours, red -> blue clockwise from 3 o'clock",
			pass < 2 ? "0" : "-4 rad");
		dump_map(s_px, TW, TH);
		/* canvas y grows downwards, and read_topdown() keeps that orientation */
		if (pass == 2) {
			ok &= expect_px("right (start -4)", 56, 33, 0xffa2005d, 24);
			ok &= expect_px("below (start -4)", 33, 56, 0xffe2001d, 24);
			ok &= expect_px("left (start -4)", 8, 33, 0xff2300dc, 24);
			ok &= expect_px("above (start -4)", 33, 8, 0xff63009c, 24);
		} else if (pass == 3) {
			ok &= expect_px("right (lut, start -4)", 56, 33, 0xff805da2, 24);
			ok &= expect_px("below (lut, start -4)", 33, 56, 0xff801de2, 24);
			ok &= expect_px("left (lut, start -4)", 8, 33, 0xff80dc23, 24);
			ok &= expect_px("above (lut, start -4)", 33, 8, 0xff809c63, 24);
			glDeleteTextures(1, &tex);
		} else if (pass == 0) {
			ok &= expect_px("right (angle 0) = red", 56, 33, 0xff0000ff, 24);
			ok &= expect_px("below (1/4) = 3/4 red", 33, 56, 0xff4000bf, 24);
			ok &= expect_px("left (1/2) = purple", 8, 33, 0xff800080, 24);
			ok &= expect_px("above (3/4) = 3/4 blue", 33, 8, 0xffbf0040, 24);
		} else {
			ok &= expect_px("right (angle 0) = ramp[0]", 56, 33, 0xff80ff00, 24);
			ok &= expect_px("below (1/4) = ramp[64]", 33, 56, 0xff80bf40, 24);
			ok &= expect_px("left (1/2) = ramp[128]", 8, 33, 0xff807f80, 24);
			ok &= expect_px("above (3/4) = ramp[192]", 33, 8, 0xff803fc0, 24);
			glDeleteTextures(1, &tex);
		}
	}
	return ok ? 0 : -1;
}

/* Case 37 "fvgstroke": a gradient paint on femtovg stroke geometry. On the
 * cluster the dial bands (10 px arc strokes with a gradient brush) came out
 * black while the same gradients fill rectangles correctly and solid strokes
 * draw. femtovg strokes are triangle strips whose tcoord.x runs 0 -> 1 across
 * the stroke (the AA fringe: strokeMask() = (1 - |2u - 1|) * strokeMult); fills
 * carry tcoord (0.5, 1). Passes: 0 fill (two triangles), 1 fill (strip), 2 a
 * 20 px stroke band with the fringe coordinates, all with fill_gradient.frag
 * (femtovg's "large box" linear gradient parameters: red until the middle row,
 * then a ramp towards blue), and 3 the same band with fill_color.frag as the
 * control. Verified on silicon 2026-09-15: the gradient is identical on the
 * fill and on the stroke geometry. */
static int case_fvgstroke(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	static const struct fvg_vertex fill_tris[6] = {
		{ { 0, 0 }, { 0.5f, 1 } }, { { TW, 0 }, { 0.5f, 1 } }, { { TW, TH }, { 0.5f, 1 } },
		{ { 0, 0 }, { 0.5f, 1 } }, { { TW, TH }, { 0.5f, 1 } }, { { 0, TH }, { 0.5f, 1 } },
	};
	static const struct fvg_vertex fill_strip[4] = {
		{ { 0, 0 }, { 0.5f, 1 } }, { { 0, TH }, { 0.5f, 1 } },
		{ { TW, 0 }, { 0.5f, 1 } }, { { TW, TH }, { 0.5f, 1 } },
	};
	/* a horizontal 20 px stroke through the middle, femtovg vertex order:
	 * (p + n*w, u=0), (p - n*w, u=1) per point */
	static const struct fvg_vertex band_strip[4] = {
		{ { 0, 22 }, { 0, 1 } }, { { 0, 42 }, { 1, 1 } },
		{ { TW, 22 }, { 0, 1 } }, { { TW, 42 }, { 1, 1 } },
	};
	bool ok = true;

	for (int pass = 0; pass < 4; pass++) {
		float frag[56] = { 0 };
		GLuint vs, fs, prog, vbo;
		GLint view_loc, frag_loc, linked = 0;
		const float large = 1e5f;

		/* pt = paintMat * (fpos, 1) = (x - TW/2, y - TH/2 + large): the gradient
		 * runs along y from the top (d = 0) to the bottom (d = 1) */
		frag[3 * 4 + 0] = 1.0f;
		frag[4 * 4 + 1] = 1.0f;
		frag[5 * 4 + 0] = -(float)TW / 2.0f;
		frag[5 * 4 + 1] = large - (float)TH / 2.0f;
		frag[5 * 4 + 2] = 1.0f;
		frag[6 * 4 + 0] = 1.0f; frag[6 * 4 + 3] = 1.0f;   /* innerCol red  */
		frag[7 * 4 + 2] = 1.0f; frag[7 * 4 + 3] = 1.0f;   /* outerCol blue */
		frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;   /* scissorExt   */
		frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;   /* scissorScale */
		frag[9 * 4 + 0] = large; frag[9 * 4 + 1] = large + (float)TH / 2.0f;  /* extent */
		frag[9 * 4 + 2] = 0.0f;                                              /* radius */
		frag[9 * 4 + 3] = (float)TH;                                         /* feather */
		frag[10 * 4 + 0] = pass >= 2 ? 10.5f : 1.0f;   /* strokeMult: (20*0.5 + 0.5)/1 */
		frag[10 * 4 + 1] = -1.0f;                      /* strokeThr */
		frag[10 * 4 + 3] = pass == 3 ? 5.0f : 0.0f;    /* shaderType */

		vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
		fs = compile(GL_FRAGMENT_SHADER, pass == 3 ? gl_test_fvg_fill_color_fs_src
							 : gl_test_fvg_gradient_fs_src);
		if (!vs || !fs)
			return -1;
		prog = glCreateProgram();
		glAttachShader(prog, vs);
		glAttachShader(prog, fs);
		glBindAttribLocation(prog, 0, "vertex");
		glBindAttribLocation(prog, 1, "tcoord");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &linked);
		if (!linked) {
			pr_err("gl_test: fvgstroke pass %d: link failed\n", pass);
			return -1;
		}
		glUseProgram(prog);

		if (pass == 0)
			vbo = upload_vbo(fill_tris, sizeof(fill_tris));
		else if (pass == 1)
			vbo = upload_vbo(fill_strip, sizeof(fill_strip));
		else
			vbo = upload_vbo(band_strip, sizeof(band_strip));
		(void)vbo;
		glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);

		view_loc = glGetUniformLocation(prog, "viewSize");
		frag_loc = glGetUniformLocation(prog, "frag");
		glUniform2f(view_loc, (float)TW, (float)TH);
		glUniform4fv(frag_loc, 14, frag);

		glViewport(0, 0, TW, TH);
		glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		if (pass == 0)
			glDrawArrays(GL_TRIANGLES, 0, 6);
		else
			glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
		glFinish();
		if (!check_gl_error("fvgstroke draw"))
			return -1;

		read_topdown();
		pr_info("gl_test: fvgstroke pass %d (%s):\n", pass,
			pass == 0 ? "fill, triangles" : pass == 1 ? "fill, strip" :
			pass == 2 ? "20 px stroke band, gradient" : "20 px stroke band, solid");
		dump_map(s_px, TW, TH);
		pr_info("gl_test: fvgstroke pass %d column 32: y=8 %08x y=24 %08x y=32 %08x y=40 %08x y=56 %08x\n",
			pass, s_px[8 * TW + 32], s_px[24 * TW + 32], s_px[32 * TW + 32],
			s_px[40 * TW + 32], s_px[56 * TW + 32]);
		/* With these parameters sdroundrect() = y - TH, so the mix factor is
		 * clamp((y - TH/2) / TH): red down to the middle row, then a ramp that
		 * reaches 3/8 blue at y = 56 (the fill and the stroke must agree). */
		if (pass < 2) {
			ok &= expect_px("top = red", 32, 8, 0xff0000ff, 12);
			ok &= expect_px("y=40 = 1/8 blue", 32, 40, 0xff2100dd, 12);
			ok &= expect_px("bottom = 3/8 blue", 32, 56, 0xff61009d, 12);
		} else if (pass == 2) {
			ok &= expect_px("band centre = red", 32, 32, 0xff0000ff, 12);
			ok &= expect_px("band centre left = red", 8, 32, 0xff0000ff, 12);
			ok &= expect_px("band y=40 = 1/8 blue", 32, 40, 0xff2100dd, 12);
		} else {
			ok &= expect_px("band centre = red", 32, 32, 0xff0000ff, 24);
			ok &= expect_px("band centre left = red", 8, 32, 0xff0000ff, 24);
		}
	}
	return ok ? 0 : -1;
}

/* Case 38 "fvgstencil": femtovg's StencilStroke sequence (the default for every
 * Slint Path stroke: paint.stencil_strokes = true) with a texture-sampling
 * fragment shader. On the cluster the dial bands (conic-gradient look-up
 * texture) came out dark while the solid track drew, through the identical
 * three-pass sequence: (1) stencil INCR + colour with strokeThr = 0.998, so the
 * AA fringe fragments are discarded, (2) the fringe with stencil EQUAL 0 and
 * strokeThr = -1, (3) a colour-masked pass zeroing the stencil. Passes: 0 the
 * sequence with fill_color.frag (control), 1 the sequence with
 * fill_image_gradient_conic.frag + the 256x1 ramp, 2 a single draw of that FS
 * with the firing discard (no stencil), 3 a single draw with strokeThr = -1.
 * Passes 0 and 1 then stroke the same band green with the same three passes
 * WITHOUT clearing the stencil: femtovg relies on pass (3) having zeroed it.
 * Found on the cluster: that pass (colour mask off, discard-capable FS) did not
 * reset the stencil, so every later stencil stroke on those pixels vanished. */
static int case_fvgstencil(void)
{
	struct fvg_vertex { float xy[2]; float uv[2]; };
	static const struct fvg_vertex band_strip[4] = {
		{ { 0, 22 }, { 0, 1 } }, { { 0, 42 }, { 1, 1 } },
		{ { TW, 22 }, { 0, 1 } }, { { TW, 42 }, { 1, 1 } },
	};
	static uint8_t ramp[256 * 4];
	bool ok = true;

	for (uint32_t i = 0; i < 256; i++) {
		ramp[i * 4 + 0] = (uint8_t)i;
		ramp[i * 4 + 1] = (uint8_t)(255 - i);
		ramp[i * 4 + 2] = 128;
		ramp[i * 4 + 3] = 255;
	}

	for (int pass = 0; pass < 4; pass++) {
		float frag[56] = { 0 };
		GLuint vs, fs, prog, vbo, tex = 0;
		GLint view_loc, frag_loc, tex_loc, linked = 0;
		bool solid = pass == 0, stencil = pass < 2;

		frag[3 * 4 + 0] = 1.0f;
		frag[4 * 4 + 1] = 1.0f;
		frag[5 * 4 + 0] = -(float)TW / 2.0f;   /* conic centre = target centre */
		frag[5 * 4 + 1] = -(float)TH / 2.0f;
		frag[5 * 4 + 2] = 1.0f;
		frag[6 * 4 + 0] = 1.0f; frag[6 * 4 + 3] = 1.0f;   /* innerCol red (solid) */
		frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;   /* scissor off */
		frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;
		frag[10 * 4 + 0] = 5.5f;                       /* strokeMult (10 px stroke) */
		frag[10 * 4 + 1] = pass == 3 ? -1.0f : 0.998039f;   /* strokeThr */
		frag[10 * 4 + 3] = solid ? 5.0f : 9.0f;        /* shaderType */
		frag[13 * 4 + 0] = -1.570796f;                 /* conicStartAngle */

		vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
		fs = compile(GL_FRAGMENT_SHADER, solid ? gl_test_fvg_fill_color_fs_src
						      : gl_test_fvg_imgconic_fs_src);
		if (!vs || !fs)
			return -1;
		prog = glCreateProgram();
		glAttachShader(prog, vs);
		glAttachShader(prog, fs);
		glBindAttribLocation(prog, 0, "vertex");
		glBindAttribLocation(prog, 1, "tcoord");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &linked);
		if (!linked) {
			pr_err("gl_test: fvgstencil pass %d: link failed\n", pass);
			return -1;
		}
		glUseProgram(prog);
		vbo = upload_vbo(band_strip, sizeof(band_strip));
		(void)vbo;
		glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		if (!solid) {
			glGenTextures(1, &tex);
			glActiveTexture(GL_TEXTURE0);
			glBindTexture(GL_TEXTURE_2D, tex);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 256, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
			glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 256, 1, GL_RGBA, GL_UNSIGNED_BYTE, ramp);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
			tex_loc = glGetUniformLocation(prog, "tex");
			if (tex_loc >= 0)
				glUniform1i(tex_loc, 0);
		}
		view_loc = glGetUniformLocation(prog, "viewSize");
		frag_loc = glGetUniformLocation(prog, "frag");
		glUniform2f(view_loc, (float)TW, (float)TH);
		glUniform4fv(frag_loc, 14, frag);

		glViewport(0, 0, TW, TH);
		glEnable(GL_BLEND);
		glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
		glClearColor(0.1f, 0.1f, 0.25f, 1.0f);
		glClearStencil(0);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		if (stencil) {
			/* femtovg stencil_stroke(): interior with INCR, fringe where 0, clear */
			glEnable(GL_STENCIL_TEST);
			glStencilMask(0xff);
			glStencilFunc(GL_EQUAL, 0, 0xff);
			glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
			glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
			frag[10 * 4 + 1] = -1.0f;
			glUniform4fv(frag_loc, 14, frag);
			glStencilFunc(GL_EQUAL, 0, 0xff);
			glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
			glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
			glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
			glStencilFunc(GL_ALWAYS, 0, 0xff);
			glStencilOp(GL_ZERO, GL_ZERO, GL_ZERO);
			glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
			glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
			/* What the next stencil stroke sees: femtovg never clears the
			 * stencil, the ZERO pass above must have reset it. Draw a solid
			 * green band over the same pixels with the same three passes; if
			 * the stencil is still 1 the EQUAL-0 passes draw nothing. */
			{
				GLuint fs2 = compile(GL_FRAGMENT_SHADER, gl_test_fvg_fill_color_fs_src);
				GLuint prog2 = glCreateProgram();
				GLint l2 = 0, frag2_loc, view2_loc;

				glAttachShader(prog2, vs);
				glAttachShader(prog2, fs2);
				glBindAttribLocation(prog2, 0, "vertex");
				glBindAttribLocation(prog2, 1, "tcoord");
				glLinkProgram(prog2);
				glGetProgramiv(prog2, GL_LINK_STATUS, &l2);
				if (!l2)
					return -1;
				glUseProgram(prog2);
				frag[6 * 4 + 0] = 0.0f; frag[6 * 4 + 1] = 1.0f;   /* innerCol green */
				frag[10 * 4 + 1] = 0.998039f;
				frag[10 * 4 + 3] = 5.0f;
				frag2_loc = glGetUniformLocation(prog2, "frag");
				view2_loc = glGetUniformLocation(prog2, "viewSize");
				glUniform2f(view2_loc, (float)TW, (float)TH);
				glUniform4fv(frag2_loc, 14, frag);
				glStencilFunc(GL_EQUAL, 0, 0xff);
				glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
				glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
				frag[10 * 4 + 1] = -1.0f;
				glUniform4fv(frag2_loc, 14, frag);
				glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
				glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
				glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
				glStencilFunc(GL_ALWAYS, 0, 0xff);
				glStencilOp(GL_ZERO, GL_ZERO, GL_ZERO);
				glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
				glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
			}
			glDisable(GL_STENCIL_TEST);
		} else {
			glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
		}
		glFinish();
		glDisable(GL_BLEND);
		if (!check_gl_error("fvgstencil draw"))
			return -1;

		read_topdown();
		pr_info("gl_test: fvgstencil pass %d (%s, %s):\n", pass,
			solid ? "fill_color" : "conic look-up",
			stencil ? "stencil-stroke 3 passes" : pass == 2 ? "one draw, discard fires" : "one draw, no discard");
		dump_map(s_px, TW, TH);
		pr_info("gl_test: fvgstencil pass %d row 32: x=8 %08x x=16 %08x x=32 %08x x=48 %08x x=56 %08x | y=22 %08x y=42 %08x\n",
			pass, s_px[32 * TW + 8], s_px[32 * TW + 16], s_px[32 * TW + 32], s_px[32 * TW + 48],
			s_px[32 * TW + 56], s_px[22 * TW + 16], s_px[42 * TW + 16]);
		if (stencil) {
			/* the second stencil stroke (green) must have covered the first */
			ok &= expect_px("second stroke drawn: band left = green", 16, 32, 0xff00ff00, 12);
			ok &= expect_px("second stroke drawn: band right = green", 48, 32, 0xff00ff00, 12);
			if (!solid)
				glDeleteTextures(1, &tex);
		} else {
			/* the band runs through the conic centre: left of it the angle is
			 * pi (fraction 3/4 with the -pi/2 start), right of it 0 (1/4) */
			ok &= expect_px("band left = ramp[192]", 16, 32, 0xff803fc0, 16);
			ok &= expect_px("band right = ramp[64]", 48, 32, 0xff80bf40, 16);
			glDeleteTextures(1, &tex);
		}
	}
	return ok ? 0 : -1;
}

/* Case 39 "perf": where a UI frame's GPU time goes. Into a 1024x600 FBO,
 * timed with glFinish: femtovg's fill_color FS (it has `discard`, so the ISP
 * runs it as a punch-through pass) drawing 256 small quads, one full-screen
 * quad and 16 full-screen quads (overdraw), and the plain uniform-colour FS
 * (no discard) drawing the same 16 full-screen quads and 256 small quads.
 * The numbers separate the per-draw cost from the per-pixel cost and show
 * what punch-through adds. Always passes; the point is the log. */
static int case_perf(void)
{
	enum { W = 1024, H = 600, SMALL = 256, OVER = 16, REPS = 5 };
	struct fvg_vertex { float xy[2]; float uv[2]; };
	struct pos_vertex { float xyzw[4]; float rgba[4]; };
	static struct fvg_vertex small_fvg[SMALL * 6], full_fvg[6];
	static struct pos_vertex small_pos[SMALL * 6], full_pos[6], full_pos16[OVER * 6], part_pos[6];
	float frag[56] = { 0 };
	GLuint tex, fbo, vs, fs, prog_a, prog_b, vbo_sf, vbo_ff, vbo_sp, vbo_fp, vbo_f16, vbo_pp;
	GLint linked = 0, loc, u_color;
	GLenum status;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: perf: FBO incomplete 0x%x\n", status);
		return -1;
	}
	glViewport(0, 0, W, H);

	/* geometry: a 16x16 grid of 16 px quads (canvas px for femtovg, NDC for
	 * the passthrough VS) and a full-screen quad */
	for (int i = 0; i < SMALL; i++) {
		float x0 = (float)(i % 16) * 60.0f + 8.0f, y0 = (float)(i / 16) * 36.0f + 8.0f;
		float x1 = x0 + 16.0f, y1 = y0 + 16.0f;
		float xs[6] = { x0, x1, x1, x0, x1, x0 }, ys[6] = { y0, y0, y1, y0, y1, y1 };

		for (int k = 0; k < 6; k++) {
			small_fvg[i * 6 + k].xy[0] = xs[k]; small_fvg[i * 6 + k].xy[1] = ys[k];
			small_fvg[i * 6 + k].uv[0] = 0.5f; small_fvg[i * 6 + k].uv[1] = 1.0f;
			small_pos[i * 6 + k].xyzw[0] = xs[k] / (W / 2.0f) - 1.0f;
			small_pos[i * 6 + k].xyzw[1] = 1.0f - ys[k] / (H / 2.0f);
			small_pos[i * 6 + k].xyzw[2] = 0.0f; small_pos[i * 6 + k].xyzw[3] = 1.0f;
			small_pos[i * 6 + k].rgba[0] = 1.0f; small_pos[i * 6 + k].rgba[3] = 1.0f;
		}
	}
	{
		float xs[6] = { 0, W, W, 0, W, 0 }, ys[6] = { 0, 0, H, 0, H, H };

		for (int k = 0; k < 6; k++) {
			full_fvg[k].xy[0] = xs[k]; full_fvg[k].xy[1] = ys[k];
			full_fvg[k].uv[0] = 0.5f; full_fvg[k].uv[1] = 1.0f;
			full_pos[k].xyzw[0] = xs[k] / (W / 2.0f) - 1.0f;
			full_pos[k].xyzw[1] = 1.0f - ys[k] / (H / 2.0f);
			full_pos[k].xyzw[2] = 0.0f; full_pos[k].xyzw[3] = 1.0f;
			full_pos[k].rgba[1] = 1.0f; full_pos[k].rgba[3] = 1.0f;
		}
	}
	/* the same full-screen quad 16 times in one buffer (one draw call), and
	 * a quad over 1/16 of the target (256x150) for the pixel-rate scaling */
	for (int k = 0; k < OVER * 6; k++)
		full_pos16[k] = full_pos[k % 6];
	{
		float xs[6] = { 0, W / 4, W / 4, 0, W / 4, 0 }, ys[6] = { 0, 0, H / 4, 0, H / 4, H / 4 };
		for (int k = 0; k < 6; k++) {
			part_pos[k] = full_pos[k];
			part_pos[k].xyzw[0] = xs[k] / (W / 2.0f) - 1.0f;
			part_pos[k].xyzw[1] = 1.0f - ys[k] / (H / 2.0f);
		}
	}
	vbo_f16 = upload_vbo(full_pos16, sizeof(full_pos16));
	vbo_pp = upload_vbo(part_pos, sizeof(part_pos));
	vbo_sf = upload_vbo(small_fvg, sizeof(small_fvg));
	vbo_ff = upload_vbo(full_fvg, sizeof(full_fvg));
	vbo_sp = upload_vbo(small_pos, sizeof(small_pos));
	vbo_fp = upload_vbo(full_pos, sizeof(full_pos));

	/* program A: femtovg main.vert + fill_color.frag (discard, never fires) */
	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	fs = compile(GL_FRAGMENT_SHADER, gl_test_fvg_fill_color_fs_src);
	if (!vs || !fs)
		return -1;
	prog_a = glCreateProgram();
	glAttachShader(prog_a, vs);
	glAttachShader(prog_a, fs);
	glBindAttribLocation(prog_a, 0, "vertex");
	glBindAttribLocation(prog_a, 1, "tcoord");
	glLinkProgram(prog_a);
	glGetProgramiv(prog_a, GL_LINK_STATUS, &linked);
	if (!linked)
		return -1;
	/* program B: passthrough.vert + uniform_color.frag (no discard) */
	prog_b = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog_b)
		return -1;

	frag[6 * 4 + 0] = 0.3f; frag[6 * 4 + 3] = 0.5f;   /* innerCol: translucent */
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;
	frag[10 * 4 + 0] = 1.0f;
	frag[10 * 4 + 1] = -1.0f;
	frag[10 * 4 + 3] = 5.0f;

	glEnable(GL_BLEND);
	glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);

	/* Each probe: which program, which VBO, draws per frame and vertices per
	 * draw, and the GL state that isolates one cost. The CPU clock brackets
	 * glDrawArrays..glFinish, so a frame's time is the kick's GPU time plus
	 * the submit/fence round trip. */
	static const struct probe {
		const char *name;
		bool fvg;        /* program A (femtovg fill_color, discard) else B */
		int vbo;         /* 0 small, 1 full, 2 full x16, 3 quarter */
		int draws, verts;
		bool blend, mask_off;
	} probes[] = {
		{ "fill_color (discard): 256 small quads, 256 draws",             true,  0, SMALL, 6,        true,  false },
		{ "fill_color (discard): 1 full-screen quad",                     true,  1, 1,     6,        true,  false },
		{ "fill_color (discard): 16 full-screen quads (overdraw 16)",     true,  1, OVER,  6,        true,  false },
		{ "uniform_color (no discard): 16 full-screen quads (overdraw 16)", false, 1, OVER, 6,       true,  false },
		{ "uniform_color (no discard): 256 small quads, 256 draws",       false, 0, SMALL, 6,        true,  false },
		{ "uniform_color, blend off: 16 full-screen quads",               false, 1, OVER,  6,        false, false },
		{ "uniform_color, colour mask off (ISP only): 16 full-screen quads", false, 1, OVER, 6,      false, true  },
		{ "uniform_color: 16 full-screen quads in ONE draw",              false, 2, 1,     OVER * 6, true,  false },
		{ "uniform_color: 256 small quads in ONE draw",                   false, 0, 1,     SMALL * 6, true, false },
		{ "clear only, no draws",                                         false, 1, 0,     6,        true,  false },
		{ "uniform_color: 16 quarter-size quads (1/16 of the pixels)",    false, 3, OVER,  6,        true,  false },
	};

	for (int t = 0; t < (int)(sizeof(probes) / sizeof(probes[0])); t++) {
		const struct probe *pb = &probes[t];
		bool fvg = pb->fvg;
		bool small = pb->vbo == 0;
		uint32_t t0, t1, best = 0xffffffffu, total = 0;
		uint32_t pc0[7], pc1[7];

		glUseProgram(fvg ? prog_a : prog_b);
		if (fvg) {
			glBindBuffer(GL_ARRAY_BUFFER, small ? vbo_sf : vbo_ff);
			glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
			glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
			loc = glGetUniformLocation(prog_a, "viewSize");
			glUniform2f(loc, (float)W, (float)H);
			loc = glGetUniformLocation(prog_a, "frag");
			glUniform4fv(loc, 14, frag);
		} else {
			GLuint vbos[4] = { vbo_sp, vbo_fp, vbo_f16, vbo_pp };

			glBindBuffer(GL_ARRAY_BUFFER, vbos[pb->vbo]);
			glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pos_vertex), (const void *)0);
			glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pos_vertex), (const void *)16);
			u_color = glGetUniformLocation(prog_b, "u_color");
			glUniform4f(u_color, 0.0f, 0.3f, 0.0f, 0.5f);
		}
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		if (pb->blend)
			glEnable(GL_BLEND);
		else
			glDisable(GL_BLEND);
		glColorMask(!pb->mask_off, !pb->mask_off, !pb->mask_off, !pb->mask_off);

		for (int r = 0; r < REPS + 1; r++) {
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			if (r == 1)
				gl_test_perf_read(pc0);
			t0 = now_ms();
			for (int i = 0; i < pb->draws; i++)
				glDrawArrays(GL_TRIANGLES, (small && pb->draws > 1) ? i * 6 : 0, pb->verts);
			glFinish();
			t1 = now_ms();
			if (r == 0)
				continue;   /* warm-up: shader upload etc. */
			total += t1 - t0;
			if (t1 - t0 < best)
				best = t1 - t0;
		}
		glColorMask(1, 1, 1, 1);
		if (!check_gl_error("perf draw"))
			return -1;
		gl_test_perf_read(pc1);
		pr_info("gl_test: perf %d: %s: best %u ms, mean %u ms per frame\n", t, pb->name,
			best, total / REPS);
		/* GPU-side attribution per frame: busy cycles at 800 MHz -> us */
		pr_info("gl_test:   per frame: TA busy %u us, 3D busy %u us, TA|3D %u us, 3D spin-up %u us, SLC0 stalls rd %u wr %u, wall %u us\n",
			(pc1[0] - pc0[0]) / 800u / REPS, (pc1[1] - pc0[1]) / 800u / REPS,
			(pc1[2] - pc0[2]) / 800u / REPS, (pc1[5] - pc0[5]) / 800u / REPS,
			(pc1[3] - pc0[3]) / REPS, (pc1[4] - pc0[4]) / REPS,
			(uint32_t)(((uint64_t)(pc1[6] - pc0[6]) * 256u) / 800u / REPS));
	}
	glDisable(GL_BLEND);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	return 0;
}

/* ---- case 40: a Qt Quick merged strip: more VS inputs than outputs -------- */

/* Qt Quick's batch renderer merges quads into one triangle strip joined by
 * repeated indices (0 1 2 3 3 4 4 5 6 7 ...), draws it front-to-back with
 * depth LESS, and gives every vertex its element's z through a third
 * attribute (a float stream after the vertex data in the same VBO). Its
 * batchable vertex shaders therefore have more input dwords (9: pos4 +
 * colour4 + z) than output dwords (8: position + colour). On silicon the
 * elements from vertex 16 on were never drawn: VDMCTRL_VDM_STATE5
 * vs_usc_unified_size was sized from the VS output, and the ninth input of
 * every instance past the first USC task (vs_max_instances 16) was lost, so
 * those vertices got a garbage z (pvr_vdm_ppp_gen.c, vs_input_size_bytes).
 * Six 8-px quads, 24 vertices, zorder.vert: every quad must show, from
 * client-side u16 indices and from a u16 IBO. */
static int case_qtstrip(void)
{
	static struct { struct cuv_vertex v[24]; float z[24]; } buf;
	static const uint16_t strip[34] = {
		0, 1, 2, 3, 3, 4, 4, 5, 6, 7, 7, 8, 8, 9, 10, 11, 11, 12,
		12, 13, 14, 15, 15, 16, 16, 17, 18, 19, 19, 20, 20, 21, 22, 23,
	};
	static const float qc[6][3] = {
		{ 1, 0, 0 }, { 0, 1, 0 }, { 0, 0, 1 }, { 1, 1, 0 }, { 0, 1, 1 }, { 1, 0, 1 },
	};
	/* read-back pixels are 0xAABBGGRR */
	static const uint32_t want[6] = { 0xff0000ff, 0xff00ff00, 0xffff0000, 0xff00ffff, 0xffffff00, 0xffff00ff };
	GLuint prog, ibo = 0;
	bool ok = true;

	prog = link_program(gl_test_zorder_vs_src, gl_test_interp_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int q = 0; q < 6; q++) {
		/* quad q: pixels [10q+2, 10q+10) x [16, 48); z 0.0 .. 0.75 front to back */
		float x0 = ndc_x((uint32_t)(10 * q + 2)) - 0.5f / (TW / 2.0f);
		float x1 = ndc_x((uint32_t)(10 * q + 10)) - 0.5f / (TW / 2.0f);

		cuv_set(&buf.v[q * 4 + 0], x0, 0.5f, 0, qc[q][0], qc[q][1], qc[q][2]);
		cuv_set(&buf.v[q * 4 + 1], x0, -0.5f, 0, qc[q][0], qc[q][1], qc[q][2]);
		cuv_set(&buf.v[q * 4 + 2], x1, 0.5f, 0, qc[q][0], qc[q][1], qc[q][2]);
		cuv_set(&buf.v[q * 4 + 3], x1, -0.5f, 0, qc[q][0], qc[q][1], qc[q][2]);
		for (int i = 0; i < 4; i++)
			buf.z[q * 4 + i] = 0.15f * (float)q;
	}
	upload_vbo(&buf, sizeof(buf));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)16);
	glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, sizeof(float), (const void *)sizeof(buf.v));
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClearDepthf(1.0f);

	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawElements(GL_TRIANGLE_STRIP, 34, GL_UNSIGNED_SHORT, strip);
	glFinish();
	if (!check_gl_error("client strip"))
		return -1;
	read_topdown();
	for (int q = 0; q < 6; q++) {
		char what[GL_TEST_LABEL];

		snprintf(what, sizeof(what), "client strip: quad %d", q);
		ok &= expect_px(what, (uint32_t)(10 * q + 6), 32, want[q], 0);
	}
	ok &= expect_px("client strip: gap stays clear", 11, 32, 0xff000000, 0);

	glGenBuffers(1, &ibo);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(strip), strip, GL_STATIC_DRAW);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawElements(GL_TRIANGLE_STRIP, 34, GL_UNSIGNED_SHORT, (const void *)0);
	glFinish();
	if (!check_gl_error("ibo strip"))
		return -1;
	read_topdown();
	for (int q = 0; q < 6; q++) {
		char what[GL_TEST_LABEL];

		snprintf(what, sizeof(what), "ibo strip: quad %d", q);
		ok &= expect_px(what, (uint32_t)(10 * q + 6), 32, want[q], 0);
	}

	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &ibo);
	glDisableVertexAttribArray(2);
	glDisable(GL_DEPTH_TEST);
	return ok ? 0 : -1;
}


/* ---- case 41/42: OpenGL ES 3.0 cube maps and 2D texture arrays ------------ */
/* YakoGL keeps a cube map's six faces / an array's layers stacked in one
 * linear texture and rewrites the shader to pick the face/layer (the
 * translator's emulation). The cases upload distinct solid colours per
 * face/layer, sample each through the GLSL ES 3.00 shaders and render into
 * one face/layer through an FBO attachment. */

/* The cases never delete their shader and program objects and the pools
 * (128 shaders, 64 programs) are full by the time the ES 3.0 cases run:
 * they free everything first (the offline backend deletes immediately). */
static void free_all_objects(void)
{
	glUseProgram(0);
	for (GLuint h = 1; h < 128; h++) {
		if (glIsProgram(h))
			glDeleteProgram(h);
		if (glIsShader(h))
			glDeleteShader(h);
	}
	while (glGetError() != GL_NO_ERROR)
		;
}
static uint32_t rgba_u32(uint8_t r, uint8_t g, uint8_t b, uint8_t a)
{
	return ((uint32_t)a << 24) | ((uint32_t)b << 16) | ((uint32_t)g << 8) | r;
}

static void fill_texels(uint8_t *dst, uint32_t n, uint8_t r, uint8_t g, uint8_t b, uint8_t a)
{
	for (uint32_t i = 0; i < n; i++) {
		dst[i * 4 + 0] = r; dst[i * 4 + 1] = g; dst[i * 4 + 2] = b; dst[i * 4 + 3] = a;
	}
}

static int case_es3cube(void)
{
	static const uint8_t face_rgb[6][3] = { { 255, 0, 0 }, { 0, 255, 0 }, { 0, 0, 255 },
						 { 255, 255, 0 }, { 255, 0, 255 }, { 0, 255, 255 } };
	static const float axes[6][3] = { { 1, 0, 0 }, { -1, 0, 0 }, { 0, 1, 0 }, { 0, -1, 0 }, { 0, 0, 1 }, { 0, 0, -1 } };
	static uint8_t texels[8 * 8 * 4];
	GLuint prog, tex = 0, fbo = 0;
	GLint axis_loc;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_cube_vs_src, gl_test_es300_cube_fs_src);
	if (!prog)
		return -1;
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	for (int f = 0; f < 6; f++) {
		fill_texels(texels, 64, face_rgb[f][0], face_rgb[f][1], face_rgb[f][2], 255);
		glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	}
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("cube upload"))
		return -1;
	glUseProgram(prog);
	axis_loc = glGetUniformLocation(prog, "axis");
	glUniform1i(glGetUniformLocation(prog, "cube"), 0);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	for (int f = 0; f < 6; f++) {
		char what[GL_TEST_LABEL];
		glUniform3f(axis_loc, axes[f][0], axes[f][1], axes[f][2]);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("cube draw"))
			return -1;
		read_topdown();
		snprintf(what, sizeof(what), "face %d centre", f);
		ok &= expect_px(what, 32, 32, rgba_u32(face_rgb[f][0], face_rgb[f][1], face_rgb[f][2], 255), 2);
		/* (16,16): NDC -0.5 -> a 0.4 offset that stays on the face (0.8 at
		 * the corner leaves it: the major axis flips to the neighbour) */
		snprintf(what, sizeof(what), "face %d off-centre", f);
		ok &= expect_px(what, 16, 16, rgba_u32(face_rgb[f][0], face_rgb[f][1], face_rgb[f][2], 255), 2);
	}
	/* render into the +Y face (white), then sample it again */
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_Y, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: cube face FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, 8, 8);
	glClearColor(1.0f, 1.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	for (int f = 0; f < 6; f += 2) {   /* +X, +Y (now white), +Z */
		char what[GL_TEST_LABEL];
		uint32_t want = f == 2 ? 0xffffffffu : rgba_u32(face_rgb[f][0], face_rgb[f][1], face_rgb[f][2], 255);
		glUniform3f(axis_loc, axes[f][0], axes[f][1], axes[f][2]);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		snprintf(what, sizeof(what), "after FBO: face %d", f);
		ok &= expect_px(what, 32, 32, want, 2);
	}
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("es3cube teardown") ? 0 : -1;
}

static int case_es3array(void)
{
	static uint8_t texels[3][8 * 8 * 4];
	GLuint prog, tex = 0, fbo = 0;
	GLint layer_loc;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_cube_vs_src, gl_test_es300_array_fs_src);
	if (!prog)
		return -1;
	for (int l = 0; l < 3; l++)
		fill_texels(texels[l], 64, (uint8_t)(l * 80), (uint8_t)(255 - l * 80), (uint8_t)(l * 40), (uint8_t)(100 + l * 50));
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
	glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA8, 8, 8, 3, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("array upload"))
		return -1;
	glUseProgram(prog);
	layer_loc = glGetUniformLocation(prog, "layer");
	glUniform1i(glGetUniformLocation(prog, "layers"), 0);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	for (int l = 0; l < 3; l++) {
		char what[GL_TEST_LABEL];
		glUniform1f(layer_loc, (float)l);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("array draw"))
			return -1;
		read_topdown();
		snprintf(what, sizeof(what), "layer %d (rgb texture, a texelFetch)", l);
		ok &= expect_px(what, 32, 32, rgba_u32((uint8_t)(l * 80), (uint8_t)(255 - l * 80), (uint8_t)(l * 40), (uint8_t)(100 + l * 50)), 2);
	}
	/* render into layer 1 (blue), then sample layers 0..2 again */
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: array layer FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, 8, 8);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	for (int l = 0; l < 3; l++) {
		char what[GL_TEST_LABEL];
		uint32_t want = l == 1 ? rgba_u32(0, 0, 255, 255)
				     : rgba_u32((uint8_t)(l * 80), (uint8_t)(255 - l * 80), (uint8_t)(l * 40), (uint8_t)(100 + l * 50));
		glUniform1f(layer_loc, (float)l);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		snprintf(what, sizeof(what), "after FBO: layer %d", l);
		ok &= expect_px(what, 32, 32, want, 2);
	}
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("es3array teardown") ? 0 : -1;
}


/* ---- case 43: gl_VertexID ------------------------------------------------ */
/* es300/vid.vert colours vertex i by i % 4 (red, green, blue, white). The
 * quad's triangles are (0 BL, 1 BR, 2 TR) and (3 BL, 4 TR, 5 TL), so a pixel
 * near the bottom-right corner is nearly green, near the top-left nearly
 * green too (vertex 5), near the bottom-left nearly red (vertex 0). */
static int case_es3vid(void)
{
	GLuint prog;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_vid_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("vid draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("bottom-right ~ vertex 1 green", 60, 60, rgba_u32(14, 227, 14, 255), 24);
	ok &= expect_px("bottom-left ~ vertex 0 red", 4, 60, rgba_u32(237, 0, 18, 255), 24);
	ok &= expect_px("top-left ~ vertex 5 green", 4, 4, rgba_u32(36, 237, 18, 255), 24);
	return ok && check_gl_error("es3vid teardown") ? 0 : -1;
}


/* ---- case 57: gl_VertexID through an indexed draw ------------------------- */
/* Case 43 covers glDrawArrays, where the driver hands the VDM a 0,1,2,... list
 * and a sequence number would look right. ES 3.0 says gl_VertexID is the index
 * the element buffer supplied, so draw the same quad with its vertices in
 * reverse (indices 5,4,3, 2,1,0) and read the colours back: the top-left corner
 * takes vertex 5's green and the bottom-left vertex 0's red. A driver counting
 * draw position instead would put red top-left and green bottom-left.
 * Qt Quick 3D's morph targets index a vertex texture by gl_VertexID in exactly
 * this shape, so this is the case that says whether they can work. */
static int case_es3vid_indexed(void)
{
	static const GLushort idx[6] = { 5, 4, 3, 2, 1, 0 };
	GLuint prog, ibo = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_vid_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glGenBuffers(1, &ibo);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glDisable(GL_CULL_FACE);   /* the reversed order flips the winding */
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (const void *)0);
	glFinish();
	if (!check_gl_error("indexed vid draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	/* Sample well inside each triangle (the quad's diagonal runs from the
	 * top-left to the bottom-right in read_topdown coordinates), so no pixel
	 * sits on the shared edge where either triangle could own it. */
	ok &= expect_px("top-left ~ index 5 green", 5, 5, rgba_u32(36, 237, 18, 255), 48);
	ok &= expect_px("near bottom-left ~ index 0 red", 8, 56, rgba_u32(237, 0, 18, 255), 48);
	ok &= expect_px("bottom-right ~ index 1 green", 58, 58, rgba_u32(14, 227, 14, 255), 48);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	return ok && check_gl_error("es3vid_indexed teardown") ? 0 : -1;
}

/* ---- case 44: clear-only frames and RGBA16F render targets --------------- */
/* es300/vid.vert + vcolor.frag draw colours by vertex id; hdrsample.frag reads
 * the target back at a quarter scale so values above 1.0 stay in range. */
static int case_es3hdr(void)
{
	GLuint cprog, sprog, tex = 0, fbo = 0, tex8 = 0, fbo8 = 0;
	bool ok = true;

	free_all_objects();
	cprog = link_program(gl_test_es300_vid_vs_src, gl_test_es300_vcolor_fs_src);
	sprog = link_program(gl_test_es300_cube_vs_src, gl_test_es300_hdrsample_fs_src);
	if (!cprog || !sprog)
		return -1;

	/* 0. an RGBA8 FBO with nothing but a clear: does a frame with no draw
	 * reach a texture target? (the float path below shares that code) */
	glGenTextures(1, &tex8);
	glBindTexture(GL_TEXTURE_2D, tex8);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glGenFramebuffers(1, &fbo8);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo8);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex8, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: RGBA8 FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, 8, 8);
	glClearColor(0.5f, 0.25f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("rgba8 clear-only"))
		return -1;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glUseProgram(sprog);
	glUniform1i(glGetUniformLocation(sprog, "hdr"), 0);
	puv_quad_setup();
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	read_topdown();
	ok &= expect_px("rgba8 clear-only x0.25", 32, 32, rgba_u32(32, 16, 64, 64), 3);

	/* 1. the same, into an RGBA16F target, cleared above 1.0 */
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F, 8, 8, 0, GL_RGBA, GL_HALF_FLOAT, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: RGBA16F FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, 8, 8);
	glClearColor(2.0f, 0.5f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("hdr clear"))
		return -1;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glUseProgram(sprog);
	puv_quad_setup();
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("hdr sample"))
		return -1;
	read_topdown();
	ok &= expect_px("cleared 2.0/0.5/0.25 x0.25", 32, 32, rgba_u32(128, 32, 16, 64), 3);

	/* 2. a draw into it (colours by vertex id, all <= 1) then x0.25 */
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glViewport(0, 0, 8, 8);
	glUseProgram(cprog);
	puv_quad_setup();
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("hdr draw"))
		return -1;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glUseProgram(sprog);
	puv_quad_setup();
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	read_topdown();
	ok &= expect_px("drawn: bottom-right ~ green x0.25", 60, 60, rgba_u32(4, 57, 4, 64), 12);
	ok &= expect_px("drawn: bottom-left ~ red x0.25", 4, 60, rgba_u32(59, 0, 5, 64), 12);

	/* 3. a clear and a draw in one frame: the draw is scissored to the left
	 * half, so the right half must keep the clear colour */
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glViewport(0, 0, 8, 8);
	glClearColor(2.0f, 0.5f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glEnable(GL_SCISSOR_TEST);
	glScissor(0, 0, 4, 8);
	glUseProgram(cprog);
	puv_quad_setup();
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisable(GL_SCISSOR_TEST);
	glFinish();
	if (!check_gl_error("hdr clear+draw"))
		return -1;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glUseProgram(sprog);
	puv_quad_setup();
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	read_topdown();
	ok &= expect_px("clear+draw: right half = clear", 56, 32, rgba_u32(128, 32, 16, 64), 4);
	ok &= expect_px("clear+draw: left half drawn", 8, 56, rgba_u32(59, 0, 5, 64), 14);

	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glDeleteFramebuffers(1, &fbo8);
	glDeleteTextures(1, &tex8);
	return ok && check_gl_error("es3hdr teardown") ? 0 : -1;
}

/* ---- case 45: dynamic uniform indexing in a runtime loop ---------------- */
/* es300/dynidx.frag sums cols[idx + i] for i in [0, n): cols[k] is
 * (0.1k, 0, 0.2, 0.25). idx=2, n=3 -> (0.9, 0, 0.6, 0.75); idx=5, n=2 ->
 * (1.1 -> 1, 0, 0.4, 0.5). */
static int case_es3dyn(void)
{
	static const float cols[8][4] = {
		{ 0.0f, 0.0f, 0.2f, 0.25f }, { 0.1f, 0.0f, 0.2f, 0.25f },
		{ 0.2f, 0.0f, 0.2f, 0.25f }, { 0.3f, 0.0f, 0.2f, 0.25f },
		{ 0.4f, 0.0f, 0.2f, 0.25f }, { 0.5f, 0.0f, 0.2f, 0.25f },
		{ 0.6f, 0.0f, 0.2f, 0.25f }, { 0.7f, 0.0f, 0.2f, 0.25f },
	};
	GLuint prog;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_vid_vs_src, gl_test_es300_dynidx_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glUniform4fv(glGetUniformLocation(prog, "cols"), 8, &cols[0][0]);
	glUniform1i(glGetUniformLocation(prog, "idx"), 2);
	glUniform1i(glGetUniformLocation(prog, "n"), 3);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("dyn draw 1"))
		return -1;
	read_topdown();
	ok &= expect_px("idx=2 n=3 -> cols[2..4]", 32, 32, rgba_u32(230, 0, 153, 191), 3);
	glUniform1i(glGetUniformLocation(prog, "idx"), 5);
	glUniform1i(glGetUniformLocation(prog, "n"), 2);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("dyn draw 2"))
		return -1;
	read_topdown();
	ok &= expect_px("idx=5 n=2 -> cols[5..6]", 32, 32, rgba_u32(255, 0, 102, 128), 3);
	return ok && check_gl_error("es3dyn teardown") ? 0 : -1;
}

/* ---- case 46: what textureSize() reports -------------------------------- */
/* es300/texsize.frag declares `precision mediump sampler2DArray;`: GLSL ES
 * 3.00 (rev 6, section 4.5.4, p.55) gives sampler2DArray no default
 * precision ("there is no default precision qualifier for the following
 * sampler types in either the vertex or fragment language: sampler3D;
 * samplerCubeShadow; sampler2DShadow; sampler2DArray; ..."), and the online
 * front end refuses a shader without one ("this sampler type has no default
 * precision (section 4.5.4)"). */
static int case_es3size(void)
{
	static uint8_t texels[8 * 24 * 4];
	GLuint prog, t2d = 0, tarr = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_cube_vs_src, gl_test_es300_texsize_fs_src);
	if (!prog)
		return -1;
	memset(texels, 0x40, sizeof(texels));
	glGenTextures(1, &t2d);
	glBindTexture(GL_TEXTURE_2D, t2d);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 24, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glGenTextures(1, &tarr);
	glActiveTexture(GL_TEXTURE1);
	glBindTexture(GL_TEXTURE_2D_ARRAY, tarr);
	glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA8, 8, 8, 3, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glActiveTexture(GL_TEXTURE0);
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "tex"), 0);
	glUniform1i(glGetUniformLocation(prog, "arr"), 1);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("texsize draw"))
		return -1;
	read_topdown();
	/* r = 2D width 8, g = 2D height 24, b = array layer height 8, a = layers 3 */
	ok &= expect_px("textureSize 2D / array", 32, 32, rgba_u32(8, 24, 8, 3), 1);
	return ok && check_gl_error("es3size teardown") ? 0 : -1;
}

/* ---- case 47: instanced draws ------------------------------------------- */
/* On the Pi 4 (V3D) this case is refused at the draw: the backend draws one
 * instance and refuses instance_count > 1 by name ("instanced draws are not
 * implemented", counted in v3d_refused_draws), so 47 FAILs there. Until
 * 0e9008ab it failed earlier, at link (NotYet("gl_InstanceID")). Real
 * instancing (Mesa v3dx_draw.c: the instanced primitive packets and
 * SET_INSTANCEID per instance) is a separate item. */
static int case_es3inst(void)
{
	static const float quad6[6 * 4] = {
		-1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,
		-1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1,
	};
	static const float quad4[4 * 4] = { -1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1 };
	static const uint16_t idx[6] = { 0, 1, 2, 0, 2, 3 };
	/* per instance: offset (2) + colour (4) */
	static const float inst[4 * 6] = {
		-0.5f, -0.5f,  1, 0, 0, 1,
		 0.5f, -0.5f,  0, 1, 0, 1,
		-0.5f,  0.5f,  0, 0, 1, 1,
		 0.5f,  0.5f,  1, 1, 1, 1,
	};
	GLuint prog, vb6 = 0, vb4 = 0, ib = 0, ivb = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_inst_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb6);
	glBindBuffer(GL_ARRAY_BUFFER, vb6);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad6), quad6, GL_STATIC_DRAW);
	glGenBuffers(1, &vb4);
	glBindBuffer(GL_ARRAY_BUFFER, vb4);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad4), quad4, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glGenBuffers(1, &ivb);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(inst), inst, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);

	for (int pass = 0; pass < 2; pass++) {
		glBindBuffer(GL_ARRAY_BUFFER, pass == 0 ? vb6 : vb4);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
		glEnableVertexAttribArray(0);
		glBindBuffer(GL_ARRAY_BUFFER, ivb);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 24, (void *)0);
		glVertexAttribPointer(2, 4, GL_FLOAT, GL_FALSE, 24, (void *)8);
		glEnableVertexAttribArray(1);
		glEnableVertexAttribArray(2);
		glVertexAttribDivisor(1, 1);
		glVertexAttribDivisor(2, 1);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		if (pass == 0)
			glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 4);
		else
			glDrawElementsInstanced(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (void *)0, 4);
		glFinish();
		if (!check_gl_error(pass == 0 ? "arrays instanced" : "elements instanced"))
			return -1;
		read_topdown();
		ok &= expect_px(pass == 0 ? "arrays: instance 0 red" : "elements: instance 0 red", 16, 48, rgba_u32(255, 0, 0, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: instance 1 green" : "elements: instance 1 green", 48, 48, rgba_u32(0, 255, 0, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: instance 2 blue" : "elements: instance 2 blue", 16, 16, rgba_u32(0, 0, 255, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: instance 3 half white" : "elements: instance 3 half white", 48, 16, rgba_u32(128, 128, 128, 128), 3);
		ok &= expect_px(pass == 0 ? "arrays: between = clear" : "elements: between = clear", 32, 32, rgba_u32(0, 0, 0, 255), 2);
	}
	glVertexAttribDivisor(1, 0);
	glVertexAttribDivisor(2, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb6);
	glDeleteBuffers(1, &vb4);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &ivb);
	return ok && check_gl_error("es3inst teardown") ? 0 : -1;
}

/* ---- case 141: instanced draws with a divisor past 1 -----------------------
 * Case 47's program and arrays with glVertexAttribDivisor 2 on the offset and
 * the colour, 4 instances: instances 0 and 1 read element 0 (red, bottom
 * left), 2 and 3 element 1 (green, bottom right), where instance 3, drawn
 * last, is at half colour (gl_InstanceID == 3 in inst.vert). The top two
 * places stay the clear colour. So the divisor (only two places filled) and
 * gl_InstanceID on the divisor path (the bottom right half, not full green)
 * are read together, as arrays and as elements.
 * A backend that takes the divisor (the V3D: one instanced draw) passes; one
 * that takes divisor 1 only gets one draw per instance from the GL core, and
 * instance 3 is either refused (gl_InstanceID read by a draw drawn on its
 * own) or, before GlesDraw.instance_id, drawn with gl_InstanceID 0: full
 * green at the bottom right, a FAIL either way until that GPU gives the
 * instance ID. */
static int case_es3instdiv(void)
{
	static const float quad6[6 * 4] = {
		-1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,
		-1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1,
	};
	static const float quad4[4 * 4] = { -1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1 };
	static const uint16_t idx[6] = { 0, 1, 2, 0, 2, 3 };
	/* per two instances: offset (2) + colour (4) */
	static const float inst[2 * 6] = {
		-0.5f, -0.5f,  1, 0, 0, 1,
		 0.5f, -0.5f,  0, 1, 0, 1,
	};
	GLuint prog, vb6 = 0, vb4 = 0, ib = 0, ivb = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_inst_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb6);
	glBindBuffer(GL_ARRAY_BUFFER, vb6);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad6), quad6, GL_STATIC_DRAW);
	glGenBuffers(1, &vb4);
	glBindBuffer(GL_ARRAY_BUFFER, vb4);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad4), quad4, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glGenBuffers(1, &ivb);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(inst), inst, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);

	for (int pass = 0; pass < 2; pass++) {
		glBindBuffer(GL_ARRAY_BUFFER, pass == 0 ? vb6 : vb4);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
		glEnableVertexAttribArray(0);
		glBindBuffer(GL_ARRAY_BUFFER, ivb);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 24, (void *)0);
		glVertexAttribPointer(2, 4, GL_FLOAT, GL_FALSE, 24, (void *)8);
		glEnableVertexAttribArray(1);
		glEnableVertexAttribArray(2);
		glVertexAttribDivisor(1, 2);
		glVertexAttribDivisor(2, 2);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		if (pass == 0)
			glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 4);
		else
			glDrawElementsInstanced(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (void *)0, 4);
		glFinish();
		if (!check_gl_error(pass == 0 ? "arrays instanced, divisor 2" : "elements instanced, divisor 2"))
			ok = false;
		read_topdown();
		ok &= expect_px(pass == 0 ? "arrays: instances 0-1 red" : "elements: instances 0-1 red", 16, 48,
				rgba_u32(255, 0, 0, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: instance 3 half green" : "elements: instance 3 half green", 48, 48,
				rgba_u32(0, 128, 0, 128), 3);
		ok &= expect_px(pass == 0 ? "arrays: top left = clear" : "elements: top left = clear", 16, 16,
				rgba_u32(0, 0, 0, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: top right = clear" : "elements: top right = clear", 48, 16,
				rgba_u32(0, 0, 0, 255), 2);
	}
	glVertexAttribDivisor(1, 0);
	glVertexAttribDivisor(2, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb6);
	glDeleteBuffers(1, &vb4);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &ivb);
	return ok && check_gl_error("es3instdiv teardown") ? 0 : -1;
}

/* ---- case 48: 3D textures ----------------------------------------------- */
/* es300/vol.frag outputs (texture().r, textureLod().r, texelFetch().b,
 * depth / 255). Slice k holds (50k, 20k, 40k, 255). */
static int case_es3vol(void)
{
	static uint8_t vox[4 * 4 * 5 * 4];
	GLuint prog, tex = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_cube_vs_src, gl_test_es300_vol_fs_src);
	if (!prog)
		return -1;
	for (int k = 0; k < 5; k++)
		for (int i = 0; i < 16; i++) {
			uint8_t *v = &vox[(k * 16 + i) * 4];
			v[0] = (uint8_t)(50 * k); v[1] = (uint8_t)(20 * k); v[2] = (uint8_t)(40 * k); v[3] = 255;
		}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_3D, tex);
	glTexImage3D(GL_TEXTURE_3D, 0, GL_RGBA8, 4, 4, 5, 0, GL_RGBA, GL_UNSIGNED_BYTE, vox);
	/* LINEAR, because the second check below wants the blend BETWEEN two
	 * slices and NEAREST cannot produce one. The case used to set NEAREST
	 * and still expect 28, which is 0 and 50 mixed 0.55 of the way -- a
	 * value no conformant implementation returns with NEAREST, where GL
	 * selects the nearest texel on the r axis as on the others and the
	 * answer is slice 1's 50.
	 *
	 * It passed on the PowerVR because that backend has no sampler3D: the
	 * glsl2usc rewrite turns one into a grid of slices and blends adjacent
	 * ones with mix() unconditionally (tools/glsl2usc.py, "sampled with a z
	 * blend"), so it returns a blend whatever the filter says. The Vivante
	 * texture unit samples the volume itself and honours NEAREST, so it
	 * returned 50 and the check failed. The expectation was encoding one
	 * backend's emulation rather than GL.
	 *
	 * Check 1 is unaffected: it samples slice 2's exact centre, where a
	 * linear blend has weight 0, and every texel within a slice is the same
	 * colour so the s/t interpolation LINEAR now also does is a no-op. */
	glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
	glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
	glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("3d upload"))
		return -1;
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "vol"), 0);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	/* slice 2's centre */
	glUniform1f(glGetUniformLocation(prog, "zz"), 2.5f / 5.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("3d draw"))
		return -1;
	read_topdown();
	ok &= expect_px("slice 2: texture/lod/fetch/depth", 32, 32, rgba_u32(100, 100, 80, 5), 3);
	/* between slices 0 and 1 (z 0.55 of the way; 0.21 * 5 keeps the fetch
	 * index clear of the 1.0 boundary) */
	glUniform1f(glGetUniformLocation(prog, "zz"), 0.21f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	read_topdown();
	ok &= expect_px("between 0 and 1: blended", 32, 32, rgba_u32(28, 28, 40, 5), 3);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("es3vol teardown") ? 0 : -1;
}

/* ---- case 49: gl_FrontFacing -------------------------------------------- */
/* es300/facing.frag: green where gl_FrontFacing, red elsewhere. puv's quad
 * is counter-clockwise in GL window space. */
static int case_es3facing(void)
{
	GLuint prog, tex = 0, fbo = 0, sprog;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_vid_vs_src, gl_test_es300_facing_fs_src);
	sprog = link_program(gl_test_es300_cube_vs_src, gl_test_es300_hdrsample_fs_src);
	if (!prog || !sprog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	for (int cw = 0; cw < 2; cw++) {
		glFrontFace(cw ? GL_CW : GL_CCW);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("facing draw"))
			return -1;
		read_topdown();
		ok &= expect_px(cw ? "window, front CW: back (red)" : "window, front CCW: front (green)",
				32, 32, cw ? rgba_u32(255, 0, 0, 255) : rgba_u32(0, 255, 0, 255), 2);
	}
	/* an FBO (no y flip), sampled back x0.25 */
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	for (int cw = 0; cw < 2; cw++) {
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glViewport(0, 0, 8, 8);
		glUseProgram(prog);
		puv_quad_setup();
		glFrontFace(cw ? GL_CW : GL_CCW);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glViewport(0, 0, TW, TH);
		glFrontFace(GL_CCW);
		glUseProgram(sprog);
		glUniform1i(glGetUniformLocation(sprog, "hdr"), 0);
		puv_quad_setup();
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		ok &= expect_px(cw ? "fbo, front CW: back (red x0.25)" : "fbo, front CCW: front (green x0.25)",
				32, 32, cw ? rgba_u32(64, 0, 0, 64) : rgba_u32(0, 64, 0, 64), 3);
	}
	glFrontFace(GL_CCW);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("es3facing teardown") ? 0 : -1;
}

/* ---- case 51: instance/primitive volume ---------------------------------
 * A Qt Quick 3D scene like simplefog draws 2000 instances of a 4900-triangle
 * sphere in one kick; that kick never completes on this GPU. This case walks
 * the same shape up in steps, printing after every glFinish, so the console
 * shows the last size the tiler swallowed. Phase A grows the instance count of
 * one draw, phase B reaches the same triangle count with that many separate
 * draws in the same kick -- which of the two stops tells instancing apart from
 * sheer primitive volume. */
static int case_es3heavy(void)
{
	enum { G = 50, NV = G * G, NQ = (G - 1) * (G - 1), NIDX = NQ * 6, MAXI = 2048 };
	static const int counts[] = { 1, 16, 64, 256, 512, 1024, 2048 };
	static float verts[NV * 4];
	static uint16_t idx[NIDX];
	static float inst[MAXI * 6];
	GLuint prog, vb = 0, ib = 0, ivb = 0;
	uint32_t seed = 12345;

	free_all_objects();
	for (int y = 0; y < G; y++) {
		for (int x = 0; x < G; x++) {
			float *v = &verts[(y * G + x) * 4];

			v[0] = -1.0f + 2.0f * (float)x / (float)(G - 1);
			v[1] = -1.0f + 2.0f * (float)y / (float)(G - 1);
			v[2] = 0.0f;
			v[3] = 1.0f;
		}
	}
	for (int y = 0, n = 0; y < G - 1; y++) {
		for (int x = 0; x < G - 1; x++) {
			uint16_t a = (uint16_t)(y * G + x), b = (uint16_t)(a + 1);
			uint16_t c = (uint16_t)(a + G), d = (uint16_t)(c + 1);

			idx[n++] = a; idx[n++] = b; idx[n++] = d;
			idx[n++] = a; idx[n++] = d; idx[n++] = c;
		}
	}
	for (int i = 0; i < MAXI; i++) {
		float *e = &inst[i * 6];

		seed = seed * 1103515245u + 12345u;
		e[0] = (float)((seed >> 16) & 0x7ff) / 1024.0f - 1.0f;
		seed = seed * 1103515245u + 12345u;
		e[1] = (float)((seed >> 16) & 0x7ff) / 1024.0f - 1.0f;
		e[2] = 0.2f; e[3] = 0.4f; e[4] = 1.0f; e[5] = 1.0f;
	}
	prog = link_program(gl_test_es300_inst_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glGenBuffers(1, &ivb);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(inst), inst, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
	glEnableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 24, (void *)0);
	glVertexAttribPointer(2, 4, GL_FLOAT, GL_FALSE, 24, (void *)8);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
	glVertexAttribDivisor(1, 1);
	glVertexAttribDivisor(2, 1);

	for (int phase = 0; phase < 2; phase++) {
		for (unsigned int k = 0; k < sizeof(counts) / sizeof(counts[0]); k++) {
			int n = counts[k];
			uint32_t t0, t1;

			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			t0 = now_ms();
			if (phase == 0) {
				glDrawElementsInstanced(GL_TRIANGLES, NIDX, GL_UNSIGNED_SHORT,
							(void *)0, n);
			} else {
				for (int i = 0; i < n; i++) {
					glDrawElementsInstanced(GL_TRIANGLES, NIDX,
								GL_UNSIGNED_SHORT, (void *)0, 1);
				}
			}
			glFinish();
			t1 = now_ms();
			pr_info("gl_test: heavy: %s %d x %d tris = %d tris: %u ms (err 0x%x)\n",
				phase == 0 ? "instanced" : "draws", n, NQ * 2, n * NQ * 2,
				t1 - t0, glGetError());
		}
	}
	glVertexAttribDivisor(1, 0);
	glVertexAttribDivisor(2, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &ivb);
	/* The point of the case is the log above; it passes if the GPU survived. */
	return check_gl_error("es3heavy teardown") ? 0 : -1;
}

/* ---- case 52: a draw too big for one kick ---------------------------------
 * The backend splits a kick when the primitives it carries would outgrow the
 * parameter buffer, and splits a single instanced draw by instances -- each
 * chunk's per-instance stream has to start at its own first instance. Every
 * instance is drawn on top of the others with additive blending, so the pixel
 * that comes back counts them: 600 instances of 1/1000 red is 0.6, and the 300
 * that also carry green sum to the same. An instance dropped, repeated, or
 * fetched from the wrong offset moves those numbers. */
static int case_es3split(void)
{
	/* Small mesh, many instances, and a primitive budget low enough that the
	 * split happens on a workload this target's parameter buffer can hold
	 * (the shipped budget is millions of primitives; 600 instances of a big
	 * mesh stacked on 64x64 would fill any parameter buffer with per-tile
	 * lists before it proves anything about the split). */
	/* 40 instances of 98 primitives at a budget of 400 is ten chunks: enough
	 * for the split to cross blending, few enough that the parameter buffer
	 * of a bring-up build (no grow pool) holds the whole pile stacked on the
	 * same pixels. */
	enum { G = 8, NV = G * G, NQ = (G - 1) * (G - 1), NIDX = NQ * 6, NI = 40,
	       BUDGET = 400 };
	static float verts[NV * 4];
	static uint16_t idx[NIDX];
	static float inst[NI * 6];
	GLuint prog, vb = 0, ib = 0, ivb = 0;
	bool ok = true;

	free_all_objects();
	for (int y = 0; y < G; y++) {
		for (int x = 0; x < G; x++) {
			float *v = &verts[(y * G + x) * 4];

			v[0] = -1.0f + 2.0f * (float)x / (float)(G - 1);
			v[1] = -1.0f + 2.0f * (float)y / (float)(G - 1);
			v[2] = 0.0f;
			v[3] = 1.0f;
		}
	}
	for (int y = 0, n = 0; y < G - 1; y++) {
		for (int x = 0; x < G - 1; x++) {
			uint16_t a = (uint16_t)(y * G + x), b = (uint16_t)(a + 1);
			uint16_t c = (uint16_t)(a + G), d = (uint16_t)(c + 1);

			idx[n++] = a; idx[n++] = b; idx[n++] = d;
			idx[n++] = a; idx[n++] = d; idx[n++] = c;
		}
	}
	for (int i = 0; i < NI; i++) {
		float *e = &inst[i * 6];

		e[0] = 0.0f; e[1] = 0.0f;                 /* all in the same place */
		e[2] = 0.001f;                            /* red: every instance */
		e[3] = i >= NI / 2 ? 0.002f : 0.0f;       /* green: the second half */
		e[4] = 1.0f;                              /* blue: opaque, for phase A */
		e[5] = 1.0f;
	}
	prog = link_program(gl_test_es300_inst2_vs_src, gl_test_es300_vcolor_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glGenBuffers(1, &ivb);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(inst), inst, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
	glEnableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 24, (void *)0);
	glVertexAttribPointer(2, 4, GL_FLOAT, GL_FALSE, 24, (void *)8);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
	glVertexAttribDivisor(1, 1);
	glVertexAttribDivisor(2, 1);
	/* The split must not change the picture: draw the same thing with the
	 * budget off and then low, and compare the two pixels. (The shared
	 * instanced vertex shader halves gl_InstanceID 3, which restarts in every
	 * chunk, so an absolute expected colour would be arithmetic about that
	 * quirk rather than about the split.) */
	for (int phase = 0; phase < 2; phase++) {
		const int blend = phase;              /* 0: opaque, 1: additive */
		const int n = phase ? NI : 8;
		uint32_t ref = 0, got = 0;

		for (int split = 0; split < 2; split++) {
			if (blend) {
				glEnable(GL_BLEND);
				glBlendFunc(GL_ONE, GL_ONE);
			} else {
				glDisable(GL_BLEND);
			}
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			gl_test_prim_budget(split ? BUDGET : 0);
			glDrawElementsInstanced(GL_TRIANGLES, NIDX, GL_UNSIGNED_SHORT, (void *)0, n);
			glFinish();
			gl_test_prim_budget(0);
			if (!check_gl_error(split ? "split draw" : "whole draw"))
				return -1;
			read_topdown();
			if (split)
				got = s_px[32 * TW + 32];
			else
				ref = s_px[32 * TW + 32];
		}
		pr_info("gl_test: split: %s %d instances: whole 0x%08x split 0x%08x\n",
			blend ? "additive" : "opaque", n, ref, got);
		if (ref != got) {
			pr_err("gl_test:   the split changed the picture (%s)\n",
			       blend ? "additive" : "opaque");
			ok = false;
		}
		if (ref == 0xff000000u || ref == 0u) {
			pr_err("gl_test:   nothing was drawn at all (0x%08x)\n", ref);
			ok = false;
		}
	}

	glDisable(GL_BLEND);
	glVertexAttribDivisor(1, 0);
	glVertexAttribDivisor(2, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &ivb);
	return ok && check_gl_error("es3split teardown") ? 0 : -1;
}

/* ---- case 54: a chain of shrinking render targets ------------------------
 * Qt Quick 3D pre-filters an environment map into a mip chain: 512x512 down to
 * 4x4, each level its own render target, each a couple of draws. On the board
 * that chain is where reflectionprobes stops -- the geometry phase of a kick
 * completes, its fragment phase never starts, and the firmware sits idle. This
 * case walks the same ladder with plain FBOs and reports the level it stops
 * at; every level is read back, so a level that silently renders nothing shows
 * up as well. */
static int case_es3chain(void)
{
	static const float quad[6 * 4] = {
		-1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,
		-1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1,
	};
	GLuint prog, vb = 0, tex[8] = { 0 }, fbo[8] = { 0 };
	const uint32_t sizes[8] = { 512, 256, 128, 64, 32, 16, 8, 4 };
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_vs_src, gl_test_uniform_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad), quad, GL_STATIC_DRAW);
	glUseProgram(prog);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
	glEnableVertexAttribArray(0);

	for (unsigned i = 0; i < 8; i++) {
		uint32_t n = sizes[i];
		uint8_t px[4] = { 0, 0, 0, 0 };

		glGenTextures(1, &tex[i]);
		glBindTexture(GL_TEXTURE_2D, tex[i]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, (GLsizei)n, (GLsizei)n, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, NULL);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glGenFramebuffers(1, &fbo[i]);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex[i], 0);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: chain: %ux%u framebuffer incomplete\n", n, n);
			return -1;
		}
		glViewport(0, 0, (GLsizei)n, (GLsizei)n);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);   /* blue: a level that never ran stays black */
		glClear(GL_COLOR_BUFFER_BIT);
		glUniform4f(glGetUniformLocation(prog, "u_color"), 1.0f, 0.0f, 0.0f, 1.0f);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		pr_info("gl_test: chain: %ux%u drawn (err 0x%x)\n", n, n, glGetError());
		glReadPixels((GLint)(n / 2), (GLint)(n / 2), 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		if (px[0] < 250 || px[2] > 5) {
			pr_err("gl_test: chain: %ux%u centre is %u,%u,%u (want red)\n",
			       n, n, px[0], px[1], px[2]);
			ok = false;
		}
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	for (unsigned i = 0; i < 8; i++) {
		glDeleteFramebuffers(1, &fbo[i]);
		glDeleteTextures(1, &tex[i]);
	}

	/* Phase A2: six kicks into ONE plain target of each size -- the same
	 * number of small kicks as a cube has faces, without the cube. If this
	 * stalls too, the faces are not what matters. */
	for (unsigned i = 2; i < 8; i++) {
		uint32_t n = sizes[i];
		GLuint ptex = 0, pfbo = 0;

		glGenTextures(1, &ptex);
		glBindTexture(GL_TEXTURE_2D, ptex);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, (GLsizei)n, (GLsizei)n, 0, GL_RGBA,
			     GL_UNSIGNED_BYTE, NULL);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glGenFramebuffers(1, &pfbo);
		glBindFramebuffer(GL_FRAMEBUFFER, pfbo);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, ptex, 0);
		glViewport(0, 0, (GLsizei)n, (GLsizei)n);
		for (unsigned k = 0; k < 6; k++) {
			glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glUniform4f(glGetUniformLocation(prog, "u_color"), 1.0f, 0.0f, 0.0f, 1.0f);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
		}
		pr_info("gl_test: chain: plain %ux%u, six kicks (err 0x%x)\n", n, n, glGetError());
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteFramebuffers(1, &pfbo);
		glDeleteTextures(1, &ptex);
	}

	/* Phase B: the same ladder into the six faces of a cube map, first
	 * 8-bit and then half-float, which is what a reflection probe's
	 * pre-filter renders into. */
	for (unsigned pass = 0; pass < 2; pass++)
	for (unsigned i = 2; i < 8; i++) {
		const GLenum ifmt = pass ? GL_RGBA16F : GL_RGBA;
		const GLenum type = pass ? GL_HALF_FLOAT : GL_UNSIGNED_BYTE;
		uint32_t n = sizes[i];
		GLuint ctex = 0, cfbo = 0;

		glGenTextures(1, &ctex);
		glBindTexture(GL_TEXTURE_CUBE_MAP, ctex);
		for (unsigned f = 0; f < 6; f++)
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, (GLint)ifmt,
				     (GLsizei)n, (GLsizei)n, 0, GL_RGBA, type, NULL);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glGenFramebuffers(1, &cfbo);
		glBindFramebuffer(GL_FRAMEBUFFER, cfbo);
		for (unsigned f = 0; f < 6; f++) {
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
					       GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, ctex, 0);
			if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
				pr_err("gl_test: chain: cube %ux%u face %u incomplete\n", n, n, f);
				return -1;
			}
			glViewport(0, 0, (GLsizei)n, (GLsizei)n);
			glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glUniform4f(glGetUniformLocation(prog, "u_color"), 1.0f, 0.0f, 0.0f, 1.0f);
			glDrawArrays(GL_TRIANGLES, 0, 6);
		}
		glFinish();
		pr_info("gl_test: chain: cube %ux%u %s, six faces drawn (err 0x%x)\n", n, n,
			pass ? "f16" : "rgba8", glGetError());
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteFramebuffers(1, &cfbo);
		glDeleteTextures(1, &ctex);
	}
	glDeleteBuffers(1, &vb);
	return ok && check_gl_error("es3chain teardown") ? 0 : -1;
}

/* ---- case 55: two colour attachments in one render ------------------------
 * A G-buffer pass writes several attachments from one fragment shader, which
 * Qt Quick 3D's userpasses example does with three. The end-of-tile program
 * emits one surface per attachment, so the driver carries a PBE state for
 * each and picks the program that emits that many; this case writes two
 * distinct colours and reads both surfaces back. */
#ifndef GL_TEST_MRT_SAME
#define GL_TEST_MRT_SAME 0
#endif
#ifndef GL_TEST_MRT_STRIDE
#define GL_TEST_MRT_STRIDE 0u
#endif
static int case_es3mrt(void)
{
	static const float quad[6 * 3] = {
		-1, -1, 0,  1, -1, 0,  1, 1, 0,
		-1, -1, 0,  1, 1, 0,  -1, 1, 0,
	};
	GLuint prog, vb = 0, tex0 = 0, tex1 = 0, fbo = 0;
	GLint l0, l1;
	bool ok = true;
	uint8_t px[4];

	free_all_objects();
	prog = link_program(gl_test_mrt_vs_src, gl_test_mrt_fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad), quad, GL_STATIC_DRAW);
	glGenTextures(1, &tex0);
	glBindTexture(GL_TEXTURE_2D, tex0);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenTextures(1, &tex1);
	glBindTexture(GL_TEXTURE_2D, tex1);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex0, 0);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D,
			       GL_TEST_MRT_SAME ? tex0 : tex1, 0);
	if (!check_gl_error("mrt attach"))
		return -1;
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: mrt: framebuffer incomplete\n");
		return -1;
	}
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, vb);
	glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 12, (void *)0);
	glEnableVertexAttribArray(0);
	glViewport(0, 0, TW, TH);
	l0 = glGetUniformLocation(prog, "ubuf.clearColor0");
	l1 = glGetUniformLocation(prog, "ubuf.clearColor1");
	pr_info("gl_test: mrt: uniform locations %d %d\n", l0, l1);
	/* Bring-up sweep: which output-register step the second attachment's
	 * emit reads from (GL_TEST_MRT_STRIDE, 0 = the format's own count). */
#ifdef GL_TEST_PVR
	pvr_backend_set_option(PVR_BACKEND_OPT_MRT_STRIDE, GL_TEST_MRT_STRIDE);
#endif
	glUniform4f(l0, 1.0f, 0.0f, 0.0f, 1.0f);   /* attachment 0: red */
	glUniform4f(l1, 0.0f, 1.0f, 0.0f, 1.0f);   /* attachment 1: green */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("mrt draw"))
		return -1;

	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex0, 0);
	glReadPixels(TW / 2, TH / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	pr_info("gl_test: mrt: attachment 0 = %u,%u,%u,%u (want 255,0,0,255)\n",
		px[0], px[1], px[2], px[3]);
	ok &= px[0] > 250 && px[1] < 5 && px[2] < 5;
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex1, 0);
	glReadPixels(TW / 2, TH / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	pr_info("gl_test: mrt: attachment 1 = %u,%u,%u,%u (want 0,255,0,255)\n",
		px[0], px[1], px[2], px[3]);
	ok &= px[0] < 5 && px[1] > 250 && px[2] < 5;

	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex0);
	glDeleteTextures(1, &tex1);
	glDeleteBuffers(1, &vb);
	return ok && check_gl_error("es3mrt teardown") ? 0 : -1;
}


/* ---- case 58: glClear honours the scissor box ------------------------------
 *
 * The frame's background object covers the whole target and has no scissor,
 * so a scissored clear cannot be one: the GL core sends it as a full-screen
 * quad (gles_draw.clear_quad) drawn in sequence with the frame's other
 * objects. Two things to see on silicon that the host cannot: the box is
 * where GL put it, and the clear lands AFTER a draw that came before it
 * rather than under the whole frame.
 */
static void solid_quad(struct cuv_vertex q[6], float r, float g, float b, float z)
{
	static const float c[6][2] = {
		{ -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, -1 }, { 1, 1 }, { -1, 1 },
	};

	for (int i = 0; i < 6; i++)
		cuv_set(&q[i], c[i][0], c[i][1], z, r, g, b);
}

static int case_clearscis(void)
{
	struct cuv_vertex q[6];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	solid_quad(q, 0, 1, 0, 0.0f);   /* green */
	upload_vbo(q, sizeof(q));
	cuv_attribs();

	/* the whole target blue, then red inside a 32x32 box at GL (16,16) */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glEnable(GL_SCISSOR_TEST);
	glScissor(16, 16, 32, 32);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	if (!check_gl_error("scissored clear"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("inside the box", 32, 32, 0xff0000ff, 0);
	ok &= expect_px("inside, at a corner", 17, 17, 0xff0000ff, 0);
	ok &= expect_px("left of the box", 8, 32, 0xffff0000, 0);
	ok &= expect_px("right of the box", 56, 32, 0xffff0000, 0);
	ok &= expect_px("one row above the box", 32, 15, 0xffff0000, 0);
	ok &= expect_px("one row below the box", 32, 48, 0xffff0000, 0);

	/* a scissored clear AFTER a draw lands after it */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glEnable(GL_SCISSOR_TEST);
	glScissor(16, 16, 32, 32);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	if (!check_gl_error("clear between draws"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("the clear is after the draw", 32, 32, 0xff0000ff, 0);
	ok &= expect_px("the draw is everywhere else", 8, 32, 0xff00ff00, 0);

	/* an empty scissor box clears nothing at all */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glEnable(GL_SCISSOR_TEST);
	glScissor(16, 16, 0, 32);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	read_topdown();
	ok &= expect_px("an empty box clears nothing", 32, 32, 0xffff0000, 0);
	return ok && check_gl_error("clearscis") ? 0 : -1;
}

/* ---- case 59: glClear honours the write masks ------------------------------
 *
 * GL applies to a clear the same masks it applies to a draw: a buffer whose
 * mask is entirely off is not cleared at all, and a stencil clear may change
 * only the bits the write mask allows. Depth and stencil are read back
 * through a following draw, because neither buffer can be read directly.
 */
static int case_clearmask(void)
{
	struct cuv_vertex q[6];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);

	/* ---- colour: a clear with every channel masked off changes nothing */
	solid_quad(q, 0, 1, 0, 0.0f);
	upload_vbo(q, sizeof(q));
	cuv_attribs();
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glFinish();
	if (!check_gl_error("masked colour clear"))
		return -1;
	read_topdown();
	ok &= expect_px("a fully masked colour clear does nothing", 32, 32, 0xffff0000, 0);

	/* ---- depth: glDepthMask(GL_FALSE) then a depth clear changes nothing.
	 * Red is drawn at NDC z 0 (window 0.5) and owns the depth; the masked
	 * clear would put it back to 1.0 if it were applied, and the green quad
	 * behind it at z 0.5 (window 0.75) would then pass GL_LESS and win. */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClearDepthf(1.0f);
	glDepthMask(GL_TRUE);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	solid_quad(q, 1, 0, 0, 0.0f);   /* red, in front */
	upload_vbo(q, sizeof(q));
	cuv_attribs();
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDepthMask(GL_FALSE);
	glClearDepthf(1.0f);
	glClear(GL_DEPTH_BUFFER_BIT);
	glDepthMask(GL_TRUE);
	solid_quad(q, 0, 1, 0, 0.5f);   /* green, behind */
	upload_vbo(q, sizeof(q));
	cuv_attribs();
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	glDisable(GL_DEPTH_TEST);
	if (!check_gl_error("masked depth clear"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("a masked depth clear does nothing", 32, 32, 0xff0000ff, 0);

	/* ---- stencil: the write mask says which bits a clear may change. From
	 * 0x00, clearing to 0xff through a 0x0f mask leaves 0x0f, so a draw that
	 * tests EQUAL 0x0f passes and one that tests EQUAL 0xff does not. */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glStencilMask(0xff);
	glClearStencil(0x00);
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glStencilMask(0x0f);
	glClearStencil(0xff);
	glClear(GL_STENCIL_BUFFER_BIT);
	glStencilMask(0xff);
	glEnable(GL_STENCIL_TEST);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	solid_quad(q, 0, 1, 0, 0.0f);
	upload_vbo(q, sizeof(q));
	cuv_attribs();
	glStencilFunc(GL_EQUAL, 0xff, 0xff);
	glDrawArrays(GL_TRIANGLES, 0, 6);   /* must not draw */
	glFinish();
	if (!check_gl_error("masked stencil clear"))
		return -1;
	read_topdown();
	ok &= expect_px("the masked bits were not cleared", 32, 32, 0xffff0000, 0);
	glStencilFunc(GL_EQUAL, 0x0f, 0xff);
	glDrawArrays(GL_TRIANGLES, 0, 6);   /* must draw */
	glFinish();
	glDisable(GL_STENCIL_TEST);
	if (!check_gl_error("stencil read-back"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("the allowed bits were cleared", 32, 32, 0xff00ff00, 0);
	return ok && check_gl_error("clearmask") ? 0 : -1;
}

/* ---- case 60: the three texture wrap modes, per axis -----------------------
 *
 * GL_REPEAT, GL_MIRRORED_REPEAT and GL_CLAMP_TO_EDGE, which the TPU's ADDRMODE
 * calls REPEAT, FLIP and CLAMP_TO_EDGE. The quad's u runs 0..2, so the right
 * half of the screen is the second period; with a 2x2 texture and NEAREST,
 * u = 1.25 and u = 1.75 tell all three modes apart:
 *   REPEAT   column 0, column 1   (the period repeats)
 *   MIRROR   column 1, column 0   (the period is mirrored)
 *   CLAMP    column 1, column 1   (the edge texel extends)
 * The second half runs the same three modes on T with S back at REPEAT, so a
 * mode that leaked from one axis to the other -- which is what a single wrap
 * bit did -- shows as the wrong row.
 */
static GLuint make_wrap_texture(void)
{
	/* 2x2: (0,0) red, (1,0) green, (0,1) blue, (1,1) white. Row 0 is t = 0,
	 * the bottom of the image. */
	static const uint8_t texels[2 * 2 * 4] = {
		255, 0, 0, 255,   0, 255, 0, 255,
		0, 0, 255, 255,   255, 255, 255, 255,
	};
	GLuint tex = 0;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	return tex;
}

/* A full-screen quad whose u runs 0..`us` and v 0..`vs`. */
static void wrap_quad(float us, float vs)
{
	struct puv_vertex q[6];
	static const float c[6][4] = {
		{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
		{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
	};

	for (int i = 0; i < 6; i++) {
		q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1];
		q[i].pos[2] = 0.0f; q[i].pos[3] = 1.0f;
		q[i].uv[0] = c[i][2] * us; q[i].uv[1] = c[i][3] * vs;
	}
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
}

#define WRAP_RED   0xff0000ffu
#define WRAP_GREEN 0xff00ff00u
#define WRAP_BLUE  0xffff0000u

static int case_wrapmode(void)
{
	/* per mode: the two S-axis probes (columns) and the two T-axis ones (rows) */
	static const struct {
		GLenum mode;
		const char *name;
		uint32_t s_at_1_25, s_at_1_75, t_at_1_25, t_at_1_75;
	} modes[] = {
		{ GL_REPEAT,          "repeat", WRAP_RED,   WRAP_GREEN, WRAP_RED,  WRAP_BLUE },
		{ GL_MIRRORED_REPEAT, "mirror", WRAP_GREEN, WRAP_RED,   WRAP_BLUE, WRAP_RED  },
		{ GL_CLAMP_TO_EDGE,   "clamp",  WRAP_GREEN, WRAP_GREEN, WRAP_BLUE, WRAP_BLUE },
	};
	GLuint prog, tex;
	bool ok = true;

	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	tex = make_wrap_texture();

	/* S: u runs 0..2, v stays inside texel row 0 */
	wrap_quad(2.0f, 0.5f);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
	for (unsigned i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, (GLint)modes[i].mode);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("wrap S draw"))
			return -1;
		read_topdown();
		pr_info("gl_test: wrap S = %s:\n", modes[i].name);
		dump_map(s_px, TW, TH);
		ok &= expect_px("S first period", 8, 48, WRAP_RED, 0);
		ok &= expect_px("S second period, u 1.25", 40, 48, modes[i].s_at_1_25, 0);
		ok &= expect_px("S second period, u 1.75", 56, 48, modes[i].s_at_1_75, 0);
	}

	/* T: v runs 0..2, u stays inside texel column 0 */
	wrap_quad(0.5f, 2.0f);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
	for (unsigned i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, (GLint)modes[i].mode);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("wrap T draw"))
			return -1;
		read_topdown();
		pr_info("gl_test: wrap T = %s:\n", modes[i].name);
		dump_map(s_px, TW, TH);
		/* v grows upwards in GL: the top-down row is 63 - the GL row */
		ok &= expect_px("T first period", 8, 55, WRAP_RED, 0);
		ok &= expect_px("T second period, v 1.25", 8, 23, modes[i].t_at_1_25, 0);
		ok &= expect_px("T second period, v 1.75", 8, 7, modes[i].t_at_1_75, 0);
	}
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("wrapmode") ? 0 : -1;
}

/* ---- case 61: GL's incomplete texture --------------------------------------
 *
 * GL ES 2.0 3.7.10: a texture that is not complete samples (0,0,0,1)
 * everywhere, whatever its texels hold. A minification filter that uses
 * mipmaps needs the whole chain; without it the texture is incomplete, and so
 * is texture object 0, which is what a sampler with nothing bound reads.
 */
static int case_texincomp(void)
{
	GLuint prog, tex;
	bool ok = true;

	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	wrap_quad(1.0f, 1.0f);
	tex = make_wrap_texture();       /* one level, NEAREST both ways */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);

	/* complete: the texels are what comes back */
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("complete draw"))
		return -1;
	read_topdown();
	ok &= expect_px("complete: the texel", 16, 48, WRAP_RED, 0);

	/* a mipmap minification filter with no chain: incomplete */
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("incomplete draw"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("incomplete: opaque black", 16, 48, 0xff000000, 0);
	ok &= expect_px("incomplete: opaque black", 48, 16, 0xff000000, 0);

	/* a filter without mipmaps makes it complete again */
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	read_topdown();
	ok &= expect_px("complete again", 16, 48, WRAP_RED, 0);

	/* nothing bound: texture object 0 is incomplete too */
	glBindTexture(GL_TEXTURE_2D, 0);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("unbound draw"))
		return -1;
	read_topdown();
	ok &= expect_px("nothing bound: opaque black", 16, 48, 0xff000000, 0);
	ok &= expect_px("nothing bound: opaque black", 48, 16, 0xff000000, 0);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("texincomp") ? 0 : -1;
}


/* ---- case 62: which depth-clip convention does the TA use? -----------------
 *
 * GL ES 2.0 2.11 clips a primitive against all six planes of the view volume,
 * near and far among them: -w <= z <= w. The backend can ask the TA for that
 * (PVR_BACKEND_OPT_DEPTH_CLIP -> clip_mode FRONT_REAR), but on silicon that
 * mode over-clips: dEQP's clip_z cases come back as a sliver of the
 * reference, which is the shape you get if the hardware clips 0 <= z <= w,
 * the D3D convention, where GL wants -w <= z <= w. So near/far clipping is
 * OFF by default and this case is what decides whether it can be turned on.
 *
 * It asks one question the answer to which cannot be argued with: with the
 * clipper ON, does a quad at NDC z -0.5 -- comfortably inside GL's volume,
 * outside a 0..w one -- still get drawn?
 *
 *   drawn      the TA clips in GL's convention; the over-clipping is
 *              something else and FRONT_REAR can go back on
 *   not drawn  the TA clips 0 <= z <= w; GL's near plane needs the z range
 *              mapped before the clipper sees it, and turning FRONT_REAR on
 *              as it stands would throw away half of every 3D scene
 *
 * The far half is checked the same way, and a quad beyond the far plane must
 * be clipped in EITHER convention -- that is the control: if it survives, the
 * clipper is not running at all and the other two answers mean nothing.
 */
/* ---- case 64: GL_EXT_texture_format_BGRA8888 -------------------------------
 *
 * The extension adds one thing: a format whose bytes are B,G,R,A where
 * GL_RGBA's are R,G,B,A. So the check worth making is that the SAME picture
 * uploaded both ways reads back the same. A swap in the wrong direction, or
 * none at all, exchanges red and blue -- which is exactly what an absolute
 * expectation on one colour would also catch, except that this way the RGBA
 * pass calibrates the texel-to-pixel mapping instead of the test assuming it.
 *
 * glReadPixels is deliberately NOT part of this: BGRA read-back is
 * GL_EXT_read_format_bgra, a different extension this library does not
 * advertise, and IMPLEMENTATION_COLOR_READ_FORMAT stays GL_RGBA. Claiming it
 * here would turn a NotSupported dEQP case into a failing one.
 */
static int case_bgra(void)
{
	/* the 2x2 make_wrap_texture uploads -- red, green / blue, white -- with
	 * the bytes in B,G,R,A order */
	static const uint8_t bgra[2 * 2 * 4] = {
		0,   0, 255, 255,	0, 255,   0, 255,
		255, 0,   0, 255,	255, 255, 255, 255,
	};
	static const uint32_t px[4][2] = { { 16, 48 }, { 48, 48 }, { 16, 16 }, { 48, 16 } };
	uint32_t want[4];
	GLuint prog, rgba_tex, bgra_tex = 0;
	bool ok = true, saw_red = false, saw_blue = false;
	const char *ext;
	int i;

	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	wrap_quad(1.0f, 1.0f);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);

	ext = (const char *)glGetString(GL_EXTENSIONS);
	ok &= gl_test_check(ext != NULL && strstr(ext, "GL_EXT_texture_format_BGRA8888") != NULL, 1, 1);

	/* the reference: the same picture through GL_RGBA */
	rgba_tex = make_wrap_texture();
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("rgba draw"))
		return -1;
	read_topdown();
	for (i = 0; i < 4; i++) {
		want[i] = s_px[px[i][1] * TW + px[i][0]];
		saw_red |= want[i] == WRAP_RED;
		saw_blue |= want[i] == WRAP_BLUE;
	}
	/* if the sample points do not see a red texel AND a blue one, the
	 * comparison below cannot tell a red/blue swap from a correct upload */
	ok &= gl_test_check(saw_red && saw_blue, 1, 1);

	/* and through GL_BGRA_EXT, the bytes swapped to match */
	glGenTextures(1, &bgra_tex);
	glBindTexture(GL_TEXTURE_2D, bgra_tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_BGRA_EXT, 2, 2, 0, GL_BGRA_EXT, GL_UNSIGNED_BYTE, bgra);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	if (!check_gl_error("bgra upload"))
		return -1;
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("bgra draw"))
		return -1;
	read_topdown();
	for (i = 0; i < 4; i++)
		ok &= expect_px("bgra matches rgba", px[i][0], px[i][1], want[i], 0);

	glDeleteTextures(1, &bgra_tex);
	glDeleteTextures(1, &rgba_tex);
	return ok && check_gl_error("bgra") ? 0 : -1;
}

/* One reading: a quad at each of three NDC z values, depth test off, so only
 * clipping can remove it. `drawn[0]` is z -0.5 (inside GL's volume, outside a
 * 0..w one -- the question), `[1]` z +0.5 (inside either -- the sanity check)
 * and `[2]` z +2.5 (beyond far -- the control, which any running clipper
 * removes). */
static int clipz_measure(bool drawn[3])
{
	static const float z[3] = { -0.5f, 0.5f, 2.5f };
	struct cuv_vertex q[6];
	int i;

	for (i = 0; i < 3; i++) {
		solid_quad(q, 0, 1, 0, z[i]);
		upload_vbo(q, sizeof(q));
		cuv_attribs();
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("clipz draw"))
			return -1;
		read_topdown();
		drawn[i] = s_px[32 * TW + 32] == 0xff00ff00;
	}
	return 0;
}

static const char *clipz_verdict(const bool drawn[3])
{
	if (drawn[2])
		return "nothing is clipping";
	if (!drawn[1])
		return "even z +0.5 is clipped: neither convention explains it";
	return drawn[0] ? "GL convention (-w <= z <= w)" : "D3D convention (0 <= z <= w)";
}

static void clipz_report(const char *what, const bool drawn[3])
{
	pr_info("gl_test: clipz: %-24s z=-0.5 %-7s z=+0.5 %-7s z=+2.5 %-7s -> %s\n",
		what,
		drawn[0] ? "drawn" : "CLIPPED",
		drawn[1] ? "drawn" : "CLIPPED",
		drawn[2] ? "drawn" : "CLIPPED",
		clipz_verdict(drawn));
}

static int case_clipz(void)
{
	bool drawn[3];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);

#ifdef GL_TEST_PVR
	{
		/* The whole answer in one run: the CR_PPP_CTRL bit against the
		 * clip mode. 0x202 is Mesa's value, bit 0 clear -- and Mesa is
		 * a VULKAN driver, which clips 0 <= z <= w; 0x203 is ours, bit
		 * 0 set. Both keep fixed_point_format (bit 9), which our VS
		 * coordinate format needs, so the only difference between the
		 * two rows is the volume.
		 *
		 * This is the sweep z_probe ran in userspace
		 * (docs/gpu-recipes/depth-clip-convention.md). The suite
		 * carries it so the board's own output states the convention
		 * instead of pointing at a note. */
		static const uint32_t ctrl[2] = { 0x202u, 0x203u };
		bool gl_volume[2] = { false, false };
		int i;

		for (i = 0; i < 2; i++) {
			pvr_backend_set_option(PVR_BACKEND_OPT_PPP_CTRL, ctrl[i]);

			pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, PVR_DEPTH_CLIP_OFF);
			if (clipz_measure(drawn))
				goto out;
			pr_info("gl_test: clipz: ppp_ctrl 0x%03x, clipper off\n", ctrl[i]);
			clipz_report("nothing should clip", drawn);
			/* with the clipper off nothing may be removed, whatever
			 * the register says: that separates "bit 0 changed the
			 * volume" from "bit 0 broke the draw" */
			ok &= gl_test_check(drawn[0] && drawn[1] && drawn[2], 1, 1);

			pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, PVR_DEPTH_CLIP_FRONT_REAR);
			if (clipz_measure(drawn))
				goto out;
			pr_info("gl_test: clipz: ppp_ctrl 0x%03x, clipper FRONT_REAR\n", ctrl[i]);
			clipz_report("the measurement", drawn);
			gl_volume[i] = !drawn[2] && drawn[1] && drawn[0];
		}

		pr_info("gl_test: clipz: VERDICT bit 0 %s the clip volume (0x202 %s, 0x203 %s)\n",
			gl_volume[0] != gl_volume[1] ? "SELECTS" : "does NOT select",
			gl_volume[0] ? "GL" : "not GL",
			gl_volume[1] ? "GL" : "not GL");

		/* What this build ships: 0x203 with FRONT_REAR, which must clip
		 * GL's volume. z_probe measured that on silicon before
		 * PPP_CTRL_CLIPSPACE became 0x203, so this is the regression
		 * check for that constant rather than an open question. */
		ok &= gl_test_check(gl_volume[1], gl_volume[1], 1);
		ok &= gl_test_check(!gl_volume[0], gl_volume[0], 0);
	}
out:
	/* back to the built-in default (0x203, FRONT_REAR) for what follows */
	pvr_backend_set_option(PVR_BACKEND_OPT_PPP_CTRL, 0);
	pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, 0);
#else
	/* No such knobs on this GPU: measure what it does by default and say
	 * so. Only the sanity check is asserted, because a GC7000 that does
	 * not clip near/far is a gap to report, not a failure of this case. */
	if (clipz_measure(drawn))
		return -1;
	clipz_report("default configuration", drawn);
	ok &= gl_test_check(drawn[1], drawn[1], 1);
#endif
	return ok && check_gl_error("clipz") ? 0 : -1;
}

static const char *const case_names[] = {
	[0] = "triangle", [1] = "uniform", [2] = "texture", [3] = "depth", [4] = "cull",
	[5] = "scissor", [6] = "fbo", [7] = "all", [8] = "blend", [9] = "stencil",
	[10] = "prims", [11] = "persist", [12] = "stencil2", [13] = "zls", [14] = "api", [15] = "egl", [16] = "msaa", [17] = "ctx2", [18] = "image", [19] = "es100", [20] = "etc",
	[21] = "rgb565", [22] = "vao", [23] = "winding", [24] = "compact", [25] = "stress",
	[26] = "heavy", [27] = "fvgrepro", [28] = "fvgvs", [29] = "fvgft", [30] = "fvgin", [31] = "fvgcov", [32] = "fvgdis", [33] = "fvgdis2",
	[34] = "unishare", [35] = "tex1d", [36] = "fvgconic", [37] = "fvgstroke", [38] = "fvgstencil", [39] = "perf", [41] = "es3cube", [42] = "es3array", [43] = "es3vid", [46] = "es3size", [47] = "es3inst", [51] = "es3heavy", [52] = "es3split", [48] = "es3vol", [49] = "es3facing", [45] = "es3dyn", [44] = "es3hdr",
	[40] = "qtstrip", [53] = "zprobe", [54] = "es3chain", [55] = "es3mrt", [56] = "manylines", [57] = "es3vid_indexed",
	[58] = "clearscis", [59] = "clearmask", [60] = "wrapmode", [61] = "texincomp", [62] = "clipz", [64] = "bgra",
	/* A dispatched case with no row here prints as `?`, which is not
	 * cosmetic: a run request and a written prediction both quoted
	 * `gl_test: case 65 polyunit: FAIL`, a line the program could not emit.
	 * A reader grepping for it would have found nothing and concluded the
	 * case had not run. 63 was still missing when 65/66/67 were added. */
	[63] = "matattr", [65] = "polyunit", [66] = "clearvp", [67] = "fbods", [68] = "armselect", [69] = "ioswap", [70] = "loopcarry", [71] = "deadloop", [72] = "intlit", [73] = "branchlane", [74] = "ifoutput", [75] = "immneg", [76] = "boolany", [77] = "transc", [78] = "matvary", [79] = "idiv", [80] = "matdyn", [81] = "branchquad", [82] = "fragcentre", [83] = "pointcoord", [84] = "fragcoord", [85] = "cubetail", [86] = "anyall", [87] = "databreak", [88] = "rsflip", [89] = "flatlast", [90] = "churn", [91] = "hdrblend", [97] = "varyings15", [98] = "uniloop", [100] = "idxtex", [101] = "cubebias", [102] = "maskclear", [103] = "maskdepth", [104] = "divloop", [105] = "pointcoordvary", [106] = "mrt3", [107] = "clearload", [108] = "uniidx", [109] = "texlinear", [110] = "psizvpm", [111] = "points", [112] = "affectand", [113] = "cubemap", [114] = "cubelod", [115] = "texcube", [116] = "uniidxe", [117] = "uniidxf", [118] = "texmip", [119] = "pbufup", [120] = "uniidxg", [121] = "uniidxh", [122] = "uniidxi", [123] = "zsapart", [124] = "binovf", [125] = "texsubfbo", [126] = "etcmip", [127] = "lines", [128] = "uniidxj", [129] = "vstexlod", [130] = "allfeat3", [131] = "allfeat1", [132] = "narrowla", [133] = "texcubemip", [134] = "texcubethin", [135] = "texnarrow", [136] = "psizmax", [137] = "psizmin", [138] = "deriv", [140] = "tfuprobe", [141] = "es3instdiv", [142] = "unibranch", [145] = "samplecov", [148] = "texbranch", [149] = "vsunits", [150] = "shprobe", [162] = "dskeep", [163] = "drawbuf0", [164] = "bufdel", [165] = "ctxclear", [166] = "fshprobe", [167] = "stcycle", [168] = "stlayers", [169] = "dstex", [171] = "viewwin", [172] = "swapzs", [173] = "splitzs", [174] = "fwccbloss", [175] = "refusehang", [176] = "mapbuf", [177] = "zonly", [178] = "heldfree", [179] = "retile", [180] = "teardown", [181] = "fusemad", [182] = "fsat", [183] = "unipad", [184] = "depchain", [185] = "kindcost", [186] = "fcsplit", [187] = "predarm", [188] = "predif", [189] = "mulone", [190] = "blendonce", [191] = "varyfma", [192] = "predlive", [193] = "offarm", [194] = "offarmn", [195] = "ifbranch", [196] = "occl", [197] = "vtxgrid",
};

/* ---- case 67: what a framebuffer object does NOT have -----------------------
 *
 * The render target keeps a depth/stencil buffer of its own whether GL says
 * one exists or not, and its colour storage is RGBA8 whatever format the
 * application asked for. Three GL rules follow from that, and every one of
 * them was broken until 2026-09-22:
 *
 *   ES 2.0 4.1.6  with no depth buffer the depth test always passes
 *   ES 2.0 4.1.4  with no stencil buffer the stencil test always passes
 *   ES 2.0 4.3.1  a colour buffer with no alpha channel reads alpha 1
 *
 * and a fourth from the backend's frame batching: two framebuffer objects can
 * share one colour buffer and keep depth/stencil of their own, so a draw that
 * wants a different depth/stencil buffer has to end the open frame even
 * though the colour address, the size and the sample count all agree.
 *
 * dEQP's fbo.render.{stencil,resize,recreate_stencilbuffer,shared_colorbuffer}
 * are built on exactly these; this is the same thing in one case with pixel
 * read-back. Frames 3 and 4 are PowerVR-only: the GC7000 back end ignores
 * GlesDraw::target_alpha_bits and batches frames its own way
 * (src/vivante/VIVANTE_NOTES.md, "Not implemented").
 * ------------------------------------------------------------------------- */
static GLuint fbods_color_tex(GLenum format)
{
	GLuint tex = 0;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, format, TW, TH, 0, format, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	return tex;
}

static int case_fbods(void)
{
	struct cuv_vertex v[12];
	GLuint prog, fbo_a, fbo_b, tex, depth_a, depth_b;
	GLenum status;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	/* 0..5: red at z = -0.5 (near).  6..11: green at z = +0.5 (far). */
	cuv_set(&v[0], -1.0f, -1.0f, -0.5f, 1, 0, 0);
	cuv_set(&v[1], 1.0f, -1.0f, -0.5f, 1, 0, 0);
	cuv_set(&v[2], 1.0f, 1.0f, -0.5f, 1, 0, 0);
	cuv_set(&v[3], -1.0f, -1.0f, -0.5f, 1, 0, 0);
	cuv_set(&v[4], 1.0f, 1.0f, -0.5f, 1, 0, 0);
	cuv_set(&v[5], -1.0f, 1.0f, -0.5f, 1, 0, 0);
	cuv_set(&v[6], -1.0f, -1.0f, 0.5f, 0, 1, 0);
	cuv_set(&v[7], 1.0f, -1.0f, 0.5f, 0, 1, 0);
	cuv_set(&v[8], 1.0f, 1.0f, 0.5f, 0, 1, 0);
	cuv_set(&v[9], -1.0f, -1.0f, 0.5f, 0, 1, 0);
	cuv_set(&v[10], 1.0f, 1.0f, 0.5f, 0, 1, 0);
	cuv_set(&v[11], -1.0f, 1.0f, 0.5f, 0, 1, 0);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glViewport(0, 0, TW, TH);

	/* frame 1: colour only, and GL_DEPTH_TEST enabled. The near quad goes
	 * down first; the far one must still be drawn, because a framebuffer
	 * with no depth attachment has no depth test to fail. */
	tex = fbods_color_tex(GL_RGBA);
	glGenFramebuffers(1, &fbo_a);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: colour-only FBO status 0x%x\n", (unsigned)status);
		return -1;
	}
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);      /* red, near */
	glDrawArrays(GL_TRIANGLES, 6, 6);      /* green, far: must win */
	glFinish();
	if (!check_gl_error("no depth attachment"))
		return -1;
	read_topdown();
	ok &= expect_px("no depth buffer", 32, 32, 0xff00ff00, 0);

	/* frame 2: same framebuffer, and a stencil test that would reject
	 * everything if there were a stencil buffer to reject it with. */
	glEnable(GL_STENCIL_TEST);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glClearStencil(0);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);      /* red: must be drawn */
	glFinish();
	if (!check_gl_error("no stencil attachment"))
		return -1;
	read_topdown();
	ok &= expect_px("no stencil buffer", 32, 32, 0xff0000ff, 0);
	glDisable(GL_STENCIL_TEST);

#ifndef GL_TEST_VIVANTE
	/* frame 3: a GL_RGB colour attachment. The clear asks for alpha 0.25
	 * and the buffer has no alpha channel, so every pixel must read 255.
	 * (The storage is RGBA8 either way, which is how the 0.25 could ever
	 * have reached memory.) */
	{
		GLuint rgb_tex = fbods_color_tex(GL_RGB);
		GLuint rgb_fbo = 0;

		glGenFramebuffers(1, &rgb_fbo);
		glBindFramebuffer(GL_FRAMEBUFFER, rgb_fbo);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rgb_tex, 0);
		status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
		if (status != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: RGB FBO status 0x%x\n", (unsigned)status);
			return -1;
		}
		glDisable(GL_DEPTH_TEST);
		glClearColor(1.0f, 0.0f, 0.0f, 0.25f);
		glClear(GL_COLOR_BUFFER_BIT);
		glFinish();
		if (!check_gl_error("RGB clear"))
			return -1;
		read_topdown();
		ok &= expect_px("RGB target reads alpha 1", 32, 32, 0xff0000ff, 0);
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteFramebuffers(1, &rgb_fbo);
		glDeleteTextures(1, &rgb_tex);
	}

	/* frames 4 and 5: two framebuffer objects over ONE colour texture,
	 * each with a depth buffer of its own. A draws the near quad and
	 * fills its depth; B then draws the far quad with the depth test on
	 * and no clear of any kind. B's depth buffer has nothing in it, so
	 * the far quad must be drawn -- unless the backend kept A's frame
	 * open, because the colour address, size and sample count all agree,
	 * and tested against A's depth. */
	glGenRenderbuffers(1, &depth_a);
	glBindRenderbuffer(GL_RENDERBUFFER, depth_a);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);
	glGenRenderbuffers(1, &depth_b);
	glBindRenderbuffer(GL_RENDERBUFFER, depth_b);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);

	glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_a);
	glGenFramebuffers(1, &fbo_b);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo_b);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_b);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: shared-colour FBO status 0x%x\n", (unsigned)status);
		return -1;
	}

	glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClearDepthf(1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);      /* red, near, into A's depth */
	glFinish();
	if (!check_gl_error("shared colour, A"))
		return -1;
	read_topdown();
	ok &= expect_px("A drew the near quad", 32, 32, 0xff0000ff, 0);

	glBindFramebuffer(GL_FRAMEBUFFER, fbo_b);
	glDrawArrays(GL_TRIANGLES, 6, 6);      /* green, far, against B's depth */
	glFinish();
	if (!check_gl_error("shared colour, B"))
		return -1;
	read_topdown();
	ok &= expect_px("B has its own depth", 32, 32, 0xff00ff00, 0);

	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo_b);
	glDeleteRenderbuffers(1, &depth_a);
	glDeleteRenderbuffers(1, &depth_b);
#else
	(void)depth_a;
	(void)depth_b;
	(void)fbo_b;
#endif
	glDisable(GL_DEPTH_TEST);

	/* frame 6: a scissored STENCIL-ONLY clear. Case 58 covers the colour
	 * one; this one has no colour write to carry the rectangle, and dEQP's
	 * fbo.render.recreate_* and .shared_colorbuffer are built on it: clear
	 * the stencil inside a box, then draw where the stencil is still 0.
	 * The box must keep the background, and every pixel outside it must
	 * take the draw -- on both axes. */
	glBindFramebuffer(GL_FRAMEBUFFER, fbo_a);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0);
	{
		GLuint sten = 0;

		glGenRenderbuffers(1, &sten);
		glBindRenderbuffer(GL_RENDERBUFFER, sten);
		glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, TW, TH);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, sten);
		status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
		if (status != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: stencil FBO status 0x%x\n", (unsigned)status);
			return -1;
		}
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClearStencil(0);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		glEnable(GL_SCISSOR_TEST);
		glScissor(16, 16, 32, 32);
		glClearStencil(1);
		glClear(GL_STENCIL_BUFFER_BIT);
		glDisable(GL_SCISSOR_TEST);

		glEnable(GL_STENCIL_TEST);
		glStencilFunc(GL_EQUAL, 0, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		glDrawArrays(GL_TRIANGLES, 0, 6);      /* red where stencil is 0 */
		glFinish();
		glDisable(GL_STENCIL_TEST);
		if (!check_gl_error("scissored stencil clear"))
			return -1;
		read_topdown();
		dump_map(s_px, TW, TH);
		/* GL (16,16)-(47,47) is read-back rows 16..47 as well: the box is
		 * symmetric about the centre, so top-down does not move it. */
		ok &= expect_px("stencil box, centre", 32, 32, 0xffff0000, 0);
		ok &= expect_px("stencil box, corner", 17, 17, 0xffff0000, 0);
		ok &= expect_px("left of the box", 8, 32, 0xff0000ff, 0);
		ok &= expect_px("right of the box", 56, 32, 0xff0000ff, 0);
		ok &= expect_px("above the box", 32, 8, 0xff0000ff, 0);
		ok &= expect_px("below the box", 32, 56, 0xff0000ff, 0);


		/* frames 7 and 8: the same clear with a draw ahead of it, and
		 * the stencil-testing draw in the same kick (7) or in the next
		 * one (8, forced by the glFinish). 8 is the one that caught the
		 * ZLS extent: CR_ISP_ZLSCTL counts ISP tiles, and counting ZLS
		 * tiles instead stored half the buffer, so a stencil written in
		 * one kick was only half there in the next. dEQP's
		 * fbo.render.recreate_* and .shared_colorbuffer are 56 cases of
		 * exactly that. */
		for (int step = 0; step < 2; step++) {
			char what[GL_TEST_LABEL];

			glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
			glClearStencil(0);
			glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 6, 6);      /* green, an object ahead */
			glEnable(GL_SCISSOR_TEST);
			glScissor(16, 16, 32, 32);
			glClearStencil(1);
			glClear(GL_STENCIL_BUFFER_BIT);
			glDisable(GL_SCISSOR_TEST);
			glClearStencil(0);
			if (step == 1)
				glFinish();            /* the box must survive a kick */
			glEnable(GL_STENCIL_TEST);
			glStencilFunc(GL_EQUAL, 0, 0xff);
			glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
			glDrawArrays(GL_TRIANGLES, 0, 6);      /* red where stencil is 0 */
			glFinish();
			glDisable(GL_STENCIL_TEST);
			if (!check_gl_error("stencil box across a kick"))
				return -1;
			read_topdown();
			snprintf(what, sizeof(what), "%s box", step ? "across a kick" : "one kick");
			ok &= expect_px(what, 32, 32, 0xff00ff00, 0);   /* green: rejected */
			snprintf(what, sizeof(what), "%s outside", step ? "across a kick" : "one kick");
			ok &= expect_px(what, 8, 32, 0xff0000ff, 0);    /* red: drawn */
		}
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0);
		glDeleteRenderbuffers(1, &sten);
	}

	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo_a);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("fbods") ? 0 : -1;
}

/* Case 63: a matrix vertex attribute.
 *
 * GL ES 2.0 2.10.4: a mat<N> attribute is ONE attribute occupying N
 * consecutive locations, one per column, and the application calls
 * glVertexAttribPointer once per column. pco_shc cannot lower a matrix input
 * at all (pco_nir_pvfio.c:909), so the shader front end splits it into N
 * vector inputs and reassembles the matrix inside the shader; this is the
 * on-silicon check that the columns arrive in the right vertex-input
 * registers and that GL still describes one attribute.
 *
 * The colour is the matrix's FIRST ROW, one element per channel, so a column
 * that lands in the wrong register comes back as the wrong colour rather than
 * as a plausible picture. The rest of the matrix is added into the position,
 * so no element is dead code; it is all zero, so the quad is full-screen.
 */
static int matattr_one(const char *label, const char *vs_src, int cols, int rows, uint32_t want)
{
	/* pos(vec4) then `cols` columns of `rows` floats, per vertex */
	const int stride = 4 + cols * rows;
	float verts[4 * (4 + 16)];
	static const float quad[4][2] = { { -1.f, -1.f }, { 1.f, -1.f }, { -1.f, 1.f }, { 1.f, 1.f } };
	/* the first row: 0.0, 0.25, 0.5, 1.0 -- distinct per column, and exact
	 * enough in 8 bits to check with a tolerance of 2 */
	static const float row0[4] = { 0.0f, 0.25f, 0.5f, 1.0f };
	GLuint prog, vbo;
	GLint loc;
	bool ok = true;

	prog = link_program(vs_src, gl_test_matattr_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);

	for (int v = 0; v < 4; v++) {
		float *p = &verts[v * stride];

		p[0] = quad[v][0]; p[1] = quad[v][1]; p[2] = 0.0f; p[3] = 1.0f;
		for (int c = 0; c < cols; c++)
			for (int r = 0; r < rows; r++)
				p[4 + c * rows + r] = (r == 0) ? row0[c] : 0.0f;
	}
	vbo = upload_vbo(verts, (size_t)stride * 4 * sizeof(float));
	if (!vbo)
		return -1;

	/* GL names the matrix once, at the first of its locations */
	loc = glGetAttribLocation(prog, "a_xf");
	if (loc != 1) {
		pr_err("gl_test: %s: glGetAttribLocation(a_xf) = %d, expected 1 (the first column)\n", label, loc);
		ok = false;
	}
	{
		GLint n = 0;

		glGetProgramiv(prog, GL_ACTIVE_ATTRIBUTES, &n);
		if (n != 2) {
			pr_err("gl_test: %s: GL_ACTIVE_ATTRIBUTES = %d, expected 2 (a_pos and one matrix)\n", label, n);
			ok = false;
		}
	}
	{
		GLint size = 0;
		GLenum type = 0;
		char name[32] = { 0 };

		glGetActiveAttrib(prog, 1, sizeof(name), NULL, &size, &type, name);
		if (strcmp(name, "a_xf") != 0 || size != 1) {
			pr_err("gl_test: %s: glGetActiveAttrib(1) = '%s' size %d, expected 'a_xf' size 1\n",
			       label, name, size);
			ok = false;
		}
	}

	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, stride * 4, (const void *)0);
	glEnableVertexAttribArray(0);
	for (int c = 0; c < cols; c++) {
		glVertexAttribPointer((GLuint)(1 + c), rows, GL_FLOAT, GL_FALSE, stride * 4,
				      (const void *)(uintptr_t)((4 + c * rows) * 4));
		glEnableVertexAttribArray((GLuint)(1 + c));
	}

	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
	glFinish();
	if (!check_gl_error(label))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px(label, TW / 2, TH / 2, want, 2);
	ok &= expect_px(label, 2, 2, want, 2);

	for (int c = 0; c <= cols; c++)
		glDisableVertexAttribArray((GLuint)c);
	glDeleteBuffers(1, &vbo);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

static int case_matattr(void)
{
	int rc = 0;

	/* mat2's first row is only two elements, so the shader fills blue with
	 * a constant 0.5; mat3 and mat4 take their whole first row. */
	if (matattr_one("mat2", gl_test_matattr2_vs_src, 2, 2, 0xff804000) != 0)
		rc = -1;
	if (matattr_one("mat3", gl_test_matattr3_vs_src, 3, 3, 0xff804000) != 0)
		rc = -1;
	if (matattr_one("mat4", gl_test_matattr4_vs_src, 4, 4, 0xff804000) != 0)
		rc = -1;
	return rc;
}

/* ---- case 65: glPolygonOffset at ONE unit (the magnitude, not the direction)
 *
 * Case 3 already offsets a coplanar quad by +-4 units and passes on silicon,
 * so it does not discriminate how the constant is scaled: four units is far
 * from the smallest resolvable depth step under either encoding. dEQP's four
 * failing polygon_offset.*_with_units cases work at ONE unit, which is exactly
 * the boundary, and this case tests the property they test.
 *
 * From es2fPolygonOffsetTests.cpp (read 2026-09-22):
 *   UsageDisplacementTestCase (:472) draws colourless with DepthFunc GL_ALWAYS
 *   and PolygonOffset(0, 0), then green with GL_EQUAL and PolygonOffset(0, +-1),
 *   and expects a black framebuffer -- its own comment is "all fragments should
 *   have different Z => DepthFunc == GL_EQUAL fails with every fragment". One
 *   unit must move the stored depth by at least one representable step.
 *   UsagePositiveNegativeTestCase (:598) checks both directions at one unit
 *   under GL_LESS.
 *
 * So: frames 1 and 2 are the displacement test at +1 and -1, frames 3 and 4 the
 * two directions. A backend whose constant is scaled twice, or in the wrong r
 * for the bound depth format, leaves the second draw's depth equal to the
 * first and frames 1/2 show green where they should show the background.
 * ------------------------------------------------------------------------- */
static int case_polyunit(void)
{
	/* One quad, drawn twice at exactly the same z: only the offset can
	 * separate them. Zero slope, so the factor term contributes nothing.
	 * A QUAD, two triangles each: until 2026-09-24 each colour was ONE
	 * triangle (the half below the diagonal), and two of the four sample
	 * points below -- (12,50) and (40,20) -- lay outside it and read the
	 * clear colour on any GPU (the i.MX8MP's check 2, imx8mp2's analysis). */
	static const float quad[6][2] = {
		{ -0.8f, -0.8f }, { 0.8f, -0.8f }, { 0.8f, 0.8f },
		{ -0.8f, -0.8f }, { 0.8f, 0.8f }, { -0.8f, 0.8f },
	};
	struct cuv_vertex v[12];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		cuv_set(&v[i], quad[i][0], quad[i][1], 0.0f, 1, 0, 0);        /* red */
		cuv_set(&v[6 + i], quad[i][0], quad[i][1], 0.0f, 0, 1, 0);    /* green */
	}
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glEnable(GL_DEPTH_TEST);
	glEnable(GL_POLYGON_OFFSET_FILL);

	/* frames 1 and 2: the displacement test, dEQP's UsageDisplacementTestCase
	 * in its smallest form. Lay the depth down with
	 * GL_ALWAYS at no offset, then try to draw green where the depth is
	 * EQUAL after a one-unit offset. One unit must displace every fragment,
	 * so nothing green may appear. */
	for (int i = 0; i < 2; i++) {
		const float units = i ? -1.0f : 1.0f;
		char what[GL_TEST_LABEL];

		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		glDepthFunc(GL_ALWAYS);
		glPolygonOffset(0.0f, 0.0f);
		glDrawArrays(GL_TRIANGLES, 0, 6);      /* red, lays the depth */
		glDepthFunc(GL_EQUAL);
		glPolygonOffset(0.0f, units);
		glDrawArrays(GL_TRIANGLES, 6, 6);      /* green, must all fail */
		glFinish();
		if (!check_gl_error("displacement draw"))
			return -1;
		read_topdown();
		/* dEQP requires EVERY fragment to be displaced -- its expected
		 * image is entirely black, so one surviving fragment fails the
		 * case. The quad here is flat, so a uniform offset displaces all
		 * of it equally and a single sample would do; sampling four
		 * spread points instead catches a displacement that applies to
		 * part of the primitive, which one centre pixel would miss. */
		static const struct { int x, y; } pts[4] = {
			{ 32, 32 }, { 12, 50 }, { 50, 50 }, { 40, 20 },
		};

		for (unsigned k = 0; k < 4; k++) {
			snprintf(what, sizeof(what), "displacement %+.0f unit at %d,%d",
				 (double)units, pts[k].x, pts[k].y);
			/* red, not green: the second draw was displaced out of EQUAL */
			ok &= expect_px(what, pts[k].x, pts[k].y, 0xff0000ff, 0);
		}
	}

	/* frames 3 and 4: the two directions at one unit under GL_LESS. A
	 * negative offset must bring the second draw in front; a positive one
	 * must keep it behind. */
	glDepthFunc(GL_LESS);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glPolygonOffset(0.0f, 0.0f);
	glDrawArrays(GL_TRIANGLES, 0, 6);              /* red at z */
	glPolygonOffset(0.0f, -1.0f);
	glDrawArrays(GL_TRIANGLES, 6, 6);              /* green, one unit nearer */
	glFinish();
	if (!check_gl_error("negative direction"))
		return -1;
	read_topdown();
	ok &= expect_px("-1 unit is nearer: green", 32, 32, 0xff00ff00, 0);

	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glPolygonOffset(0.0f, 1.0f);
	glDrawArrays(GL_TRIANGLES, 0, 6);              /* red, one unit farther */
	glPolygonOffset(0.0f, 0.0f);
	glDrawArrays(GL_TRIANGLES, 6, 6);              /* green at z, nearer */
	glFinish();
	if (!check_gl_error("positive direction"))
		return -1;
	read_topdown();
	ok &= expect_px("+1 unit is farther: green wins", 32, 32, 0xff00ff00, 0);

	glDisable(GL_POLYGON_OFFSET_FILL);
	glDisable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	return ok ? 0 : -1;
}

/* ---- case 66: glClear is not bounded by the viewport ----------------------
 *
 * ES 2.0 3.7.3 lists what affects a clear: the scissor test, the colour mask
 * and the depth/stencil write masks. The viewport is not among them, and that
 * follows from what a viewport IS -- it transforms vertices, and a clear has
 * none of its own.
 *
 * It is worth a case because of HOW a backend implements a clear. Both of ours
 * draw a full-screen quad in clip space, and the viewport transform maps clip
 * space onto the viewport rectangle: put the clear through the application's
 * viewport and it clears only that rectangle, leaving the rest of the surface
 * holding the previous frame. That is exactly what the GC7000 backend did
 * (measured by cts-runner with driver/tools/gclab/user/clear_viewport.c: a
 * 32x32 viewport on a 64x64 surface left 3072 of 4096 pixels standing, first
 * at (32,0), on both pairs with glGetError 0).
 *
 * The failure is invisible in any single-frame test, which is why it survived:
 * every case that clears and draws at a full viewport passes. It shows up as
 * the PREVIOUS case's picture bleeding into this one, so in a suite it looks
 * like anything except a clear.
 *
 * Two steps, and the second is the whole test:
 *   1. viewport = whole surface, clear white  -> every pixel white
 *   2. viewport = a corner quarter, clear blue -> EVERY pixel blue,
 *      including the ones outside the viewport
 *
 * The scissor is explicitly disabled so that the one thing that MAY bound a
 * clear is not doing it, and a third step puts the scissor back to check the
 * other half: what must not bound a clear does not, and what must still does.
 * ------------------------------------------------------------------------- */
static int case_clearvp(void)
{
	bool ok = true;

	glDisable(GL_SCISSOR_TEST);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);

	/* step 1: the whole surface, white */
	glViewport(0, 0, TW, TH);
	glClearColor(1.0f, 1.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("clear at the full viewport"))
		return -1;
	read_topdown();
	ok &= expect_px("full-viewport clear: inside", TW / 4, TH / 4, 0xffffffff, 0);
	ok &= expect_px("full-viewport clear: far corner", TW - 1, TH - 1, 0xffffffff, 0);

	/* step 2: a quarter viewport, blue. The viewport must change nothing. */
	glViewport(0, 0, TW / 2, TH / 2);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("clear at a quarter viewport"))
		return -1;
	read_topdown();
	/* inside the viewport: blue either way, so this only says the clear ran */
	ok &= expect_px("quarter viewport: inside is cleared", TW / 4, TH / 4, 0xffff0000, 0);
	/* OUTSIDE the viewport: this is the case. White here means the clear was
	 * bounded by the viewport and the previous frame survived. */
	ok &= expect_px("outside the viewport, right", TW - 1, TH / 4, 0xffff0000, 0);
	ok &= expect_px("outside the viewport, below", TW / 4, TH - 1, 0xffff0000, 0);
	ok &= expect_px("outside the viewport, corner", TW - 1, TH - 1, 0xffff0000, 0);
	/* the first pixel past the viewport's edge, where the defect began */
	ok &= expect_px("first pixel past the edge", TW / 2, 0, 0xffff0000, 0);

	/* step 3: the scissor DOES bound a clear, and must keep doing so. Green
	 * inside the box, still blue outside it. */
	glViewport(0, 0, TW, TH);
	glEnable(GL_SCISSOR_TEST);
	glScissor(0, 0, TW / 2, TH / 2);
	glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	if (!check_gl_error("scissored clear"))
		return -1;
	read_topdown();
	/* GL's scissor counts y from the bottom; read_topdown gives a top-down
	 * image, so the box is the BOTTOM-left quarter of the picture. */
	ok &= expect_px("scissored clear: inside the box", TW / 4, TH - 1 - TH / 4, 0xff00ff00, 0);
	ok &= expect_px("scissored clear: outside the box", TW - 1, TH / 4, 0xffff0000, 0);

	return ok ? 0 : -1;
}

/* ---- case 68: one arm writes one local, the other writes another ----------
 *
 * The shape that miscompiled, drawn on the GPU:
 *
 *     vec4 v = u_a;
 *     vec4 w = u_b;
 *     if (u_flag.x > 0.5) { v = u_b; } else { w = u_a; }
 *     gl_FragColor = v * 0.5 + w * 0.5;
 *
 * With the flag set both locals hold u_b and the colour is u_b; without it
 * both hold u_a and the colour is u_a. Each arm writes ONE of the two, which
 * is what `if (c) { x = ...; } else { y = ...; }` does in any real shader.
 *
 * The in-process compiler reserved a register for the arm that does not write
 * a local and then selected from it at the join, so on one side of the
 * condition the result was whatever that register happened to hold: not u_a,
 * not u_b, and different every time the allocator moved. 30 of the 137
 * sources that compile carried it -- every FemtoVG fill, gradient, image and
 * stencil shader, which is what the UI draws with (gles2-core fce851e,
 * 2026-09-22).
 *
 * NOTHING CAUGHT IT. `gl_test` returned 25/29 with the same four failures on
 * every online board run while those shaders were being miscompiled, because
 * not one `gl_test` shader contains an `if`. This case is that gap: it is the
 * suite's only branch, and it is read back on BOTH sides of the condition,
 * because a compiler that gets one side right and the other wrong passes a
 * test that only draws once.
 *
 * The offline arm needs a blob for armselect.frag, which the shader table
 * does not have until it is rebuilt (yakogl task 39, constraint 4). Until
 * then this case is an online-path case; it is out of the regression set for
 * the same reason case 63 is.
 * ------------------------------------------------------------------------- */
static int case_armselect(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint a, b, flag;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_armselect_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	a = glGetUniformLocation(prog, "u_a");
	b = glGetUniformLocation(prog, "u_b");
	flag = glGetUniformLocation(prog, "u_flag");
	if (a < 0 || b < 0 || flag < 0) {
		pr_err("gl_test: armselect uniforms not found (u_a %d u_b %d u_flag %d)\n", a, b, flag);
		return -1;
	}
	/* u_a red, u_b blue: a wrong side is not a shade of the right answer,
	 * it is the other colour or neither. */
	glUniform4f(a, 1.0f, 0.0f, 0.0f, 1.0f);
	glUniform4f(b, 0.0f, 0.0f, 1.0f, 1.0f);

	/* THE TRUE SIDE. `w` is the local the then arm does not write, and its
	 * register is the one that used to be selected unwritten. */
	glUniform4f(flag, 1.0f, 0.0f, 0.0f, 0.0f);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("armselect, flag set"))
		return -1;
	read_topdown();
	ok &= expect_px("flag set: the colour is u_b", TW / 2, TH / 2, 0xffff0000, 2);
	ok &= expect_px("flag set: and at a corner too", 1, 1, 0xffff0000, 2);

	/* THE FALSE SIDE, the one the defect left alone: `v` keeps u_a and the
	 * else arm writes `w` with u_a as well. */
	glUniform4f(flag, 0.0f, 0.0f, 0.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("armselect, flag clear"))
		return -1;
	read_topdown();
	ok &= expect_px("flag clear: the colour is u_a", TW / 2, TH / 2, 0xff0000ff, 2);
	ok &= expect_px("flag clear: and at a corner too", 1, 1, 0xff0000ff, 2);

	return ok ? 0 : -1;
}

/* ---- case 69: an output in the register an input still occupies -----------
 *
 *     attribute vec4 a_color;   // arrives in r0
 *     attribute vec4 a_pos;     // arrives in r1
 *     v_color = a_color;        // v_color is r1
 *     gl_Position = vec4(a_pos.xy * 0.75, 0.0, 1.0);   // gl_Position is r0
 *
 * Inputs and outputs are numbered in the same register file and neither
 * numbering knows about the other. The in-process compiler emitted the copy
 * into r1 first and computed the position from r1 afterwards -- from the
 * colour. The quad covers the middle three quarters of the target, so the
 * centre is the colour and the corner is the clear; a position read from the
 * colour puts the geometry somewhere else entirely and one of the two is
 * wrong.
 *
 * The shader is not in the offline table (yakogl task 39), so this is an
 * online-path case, out of the regression set for the reason 68 is.
 * ------------------------------------------------------------------------- */
static int case_ioswap(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint lc, lp;
	bool ok = true;

	prog = link_program(gl_test_ioswap_vs_src, gl_test_interp_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		/* green: a colour whose every component differs from the
		 * position's, so neither can pass for the other */
		v[i].color[0] = 0.0f; v[i].color[1] = 1.0f; v[i].color[2] = 0.0f; v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	/* Declaration order gives a_color 0 and a_pos 1; asked rather than
	 * assumed, because the case is about which register each one lands in. */
	lc = glGetAttribLocation(prog, "a_color");
	lp = glGetAttribLocation(prog, "a_pos");
	if (lc != 0 || lp != 1) {
		pr_err("gl_test: ioswap locations a_color %d a_pos %d, expected 0 and 1\n", lc, lp);
		return -1;
	}
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer((GLuint)lc, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("ioswap"))
		return -1;
	read_topdown();
	ok &= expect_px("ioswap: the centre is the colour", TW / 2, TH / 2, 0xff00ff00, 2);
	ok &= expect_px("ioswap: inside the quad near its edge", TW / 8 + 2, TH / 8 + 2, 0xff00ff00, 2);
	ok &= expect_px("ioswap: the corner is outside the quad", 1, 1, 0xff000000, 2);
	return ok ? 0 : -1;
}

/* ---- case 70: a loop that reads values made before it ---------------------
 *
 *     float n = ceil(u_k.x * 3.0);   float k = u_k.y;   float acc = 0.0;
 *     for (float i = 0.0; i < 8.0; i += 1.0) {
 *         if (i >= n) break;
 *         acc += k;   float t = acc * 2.0;   acc = t * 0.5;
 *     }
 *     gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
 *
 * `n` and `k` are last mentioned inside the body, and `t` is made after
 * them. The in-process allocator ended a value's range at its last mention,
 * so `t` got their register and from the second trip the test and the sum
 * read `t`. Drawn at two bounds, so a loop that stopped after one trip is not
 * mistaken for one that ran three: u_k = (1, 0.25) is three trips, 0.75 red;
 * u_k = (0.34, 0.25) is two (ceil(1.02) = 2), 0.5 red.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
/* ---- case 149: sixteen texture units in the vertex stage -----------------
 *
 * dEQP-GLES2 uniform_api asks for 9..14 vertex texture units (15 cases "N vertex
 * texture units required, 8 supported" on the AM62P, RESULTS 1s), and Mesa's pvr
 * gives every stage 16 samplers (pvr_physical_device.c:634
 * maxPerStageDescriptorSamplers). This case measures what the vertex stage
 * does with sixteen, whatever GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS says: sampler
 * k (unit k) is a 1x1 texture of colour vsu_rgba(k), and draw k, with the
 * one-hot weight u_w[k], writes sampler k's texel into cell k of a 4x4 grid
 * of 16x16 cells. A unit the vertex stage does not get reads the backend's
 * black 1x1, so a short limit shows as black cells from the first unit
 * past it. */
/* The vertex shader for `n` samplers u_s0..u_s<n-1>: sampler k weighted by
 * u_w[k / 4][k % 4]. Built at run time so CONFIG_GL_TEST_VSU_UNITS can ask
 * for fewer than sixteen (the smallest count that faults, RESULTS 1t). */
static const char vsunits_vs_head[] =
	"#version 100\n"
	"attribute vec4 a_pos;\n"
	"uniform vec4 u_w[4];\n"
	"varying vec4 v_c;\n";

static void vsunits_vs_src(char *buf, size_t size, int n)
{
	static const char comp[4] = { 'x', 'y', 'z', 'w' };
	size_t len = (size_t)snprintf(buf, size, "%s", vsunits_vs_head);

	for (int k = 0; k < n && len < size; k++)
		len += (size_t)snprintf(buf + len, size - len, "uniform sampler2D u_s%d;\n", k);
	if (len < size)
		len += (size_t)snprintf(buf + len, size - len, "void main() {\n    vec2 c = vec2(0.5);\n    vec4 r = vec4(0.0);\n");
	for (int k = 0; k < n && len < size; k++)
		len += (size_t)snprintf(buf + len, size - len, "    r += u_w[%d].%c * texture2DLod(u_s%d, c, 0.0);\n", k / 4,
					comp[k % 4], k);
	if (len < size)
		(void)snprintf(buf + len, size - len, "    v_c = r;\n    gl_Position = a_pos;\n}\n");
}


/* The fragment shader with `n` samplers u_f0..u_f<n-1> as well
 * (CONFIG_GL_TEST_VSU_FS_UNITS). They are active and sampled, and u_fz = 0
 * keeps them out of the colour, so the cells are judged exactly as without
 * them. What they change is where the vertex samplers live: on the GC7000 the
 * two stages share one slot array and the vertex stage starts after the
 * fragment slots (VS_SAMPLER_BASE), so sixteen of each put vertex sampler k in
 * slot 16 + k, the upper half that no fragment-only draw reaches. */
static void vsunits_fs_samplers_src(char *buf, size_t size, int n)
{
	size_t len = (size_t)snprintf(buf, size, "#version 100\nprecision mediump float;\nvarying vec4 v_c;\n"
						 "uniform float u_fz;\n");

	for (int j = 0; j < n && len < size; j++)
		len += (size_t)snprintf(buf + len, size - len, "uniform sampler2D u_f%d;\n", j);
	if (len < size)
		len += (size_t)snprintf(buf + len, size - len, "void main() {\n    vec4 f = vec4(0.0);\n");
	for (int j = 0; j < n && len < size; j++)
		len += (size_t)snprintf(buf + len, size - len, "    f += texture2D(u_f%d, vec2(0.5));\n", j);
	if (len < size)
		(void)snprintf(buf + len, size - len, "    gl_FragColor = v_c + u_fz * f;\n}\n");
}

#define VSU_UNITS 16
#define VSU_CELL (TW / 4u)
/* The samplers the vertex shader declares and draws with (1..16); the cells
 * past it are not drawn and keep the blue clear. */
#ifdef CONFIG_GL_TEST_VSU_UNITS
#define VSU_USED CONFIG_GL_TEST_VSU_UNITS
#else
#define VSU_USED VSU_UNITS
#endif
_Static_assert(VSU_USED >= 1 && VSU_USED <= VSU_UNITS, "GL_TEST_VSU_UNITS is 1..16");
/* Fragment samplers beside them (0..16; 0 is the case as it was). Their
 * textures sit on units 16.. and are magenta, which no vertex unit's colour
 * is: a vertex sampler that read a fragment slot shows magenta. */
#ifdef CONFIG_GL_TEST_VSU_FS_UNITS
#define VSU_FS_USED CONFIG_GL_TEST_VSU_FS_UNITS
#else
#define VSU_FS_USED 0
#endif
_Static_assert(VSU_FS_USED >= 0 && VSU_FS_USED <= 16, "GL_TEST_VSU_FS_UNITS is 0..16");
#define VSU_BLUE 0xffff0000u /* the clear, as s_px holds it */

/* Unit k's colour, as s_px holds it (R in the low byte): red 8 + 16k tells
 * the units apart; green and blue are 255 - 16k and alternate 0x40 / 0xc0, so
 * no unit's colour is black or another unit's. */
static uint32_t vsu_rgba(int k)
{
	uint32_t r = 8u + 16u * (uint32_t)k, g = 255u - 16u * (uint32_t)k, b = (k & 1) ? 0xc0u : 0x40u;

	return 0xff000000u | (b << 16) | (g << 8) | r;
}

static int case_vsunits(void)
{
	static char vs_src[2048], fs_src[2048];
	GLuint prog, tex[VSU_UNITS] = { 0 }, ftex[16] = { 0 };
	GLint lp, lw, maxv = 0;
	struct puv_vertex q[6];
	bool ok = true;

	glGetIntegerv(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, &maxv);
	pr_info("gl_test: vsunits: GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS = %d; drawing with %d, %d fragment samplers beside\n",
		maxv, VSU_USED, VSU_FS_USED);
	vsunits_vs_src(vs_src, sizeof(vs_src), VSU_USED);
	if (VSU_FS_USED > 0)
		vsunits_fs_samplers_src(fs_src, sizeof(fs_src), VSU_FS_USED);
	prog = link_program(vs_src, VSU_FS_USED > 0 ? fs_src : gl_test_vsunits_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	lp = glGetAttribLocation(prog, "a_pos");
	lw = glGetUniformLocation(prog, "u_w");
	if (lp < 0 || lw < 0) {
		pr_err("vsunits: a_pos %d u_w %d\n", lp, lw);
		glDeleteProgram(prog);
		return -1;
	}
	glGenTextures(VSU_USED, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	for (int k = 0; k < VSU_USED; k++) {
		char name[8];
		uint32_t c = vsu_rgba(k);
		uint8_t texel[4] = { (uint8_t)c, (uint8_t)(c >> 8), (uint8_t)(c >> 16), (uint8_t)(c >> 24) };

		snprintf(name, sizeof(name), "u_s%d", k);
		glUniform1i(glGetUniformLocation(prog, name), k);
		glActiveTexture(GL_TEXTURE0 + (GLenum)k);
		glBindTexture(GL_TEXTURE_2D, tex[k]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, texel);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	}
	if (VSU_FS_USED > 0) {
		static const uint8_t magenta[4] = { 0xff, 0x00, 0xff, 0xff };

		glUniform1f(glGetUniformLocation(prog, "u_fz"), 0.0f);
		glGenTextures(VSU_FS_USED, ftex);
		for (int j = 0; j < VSU_FS_USED; j++) {
			char name[8];

			snprintf(name, sizeof(name), "u_f%d", j);
			glUniform1i(glGetUniformLocation(prog, name), VSU_UNITS + j);
			glActiveTexture(GL_TEXTURE0 + (GLenum)(VSU_UNITS + j));
			glBindTexture(GL_TEXTURE_2D, ftex[j]);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, magenta);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		}
	}
	glActiveTexture(GL_TEXTURE0);
	if (!check_gl_error("vsunits upload")) {
		ok = false;
		goto out;
	}

	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glEnableVertexAttribArray((GLuint)lp);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f); /* blue: "drew nothing" is not "drew black" */
	glClear(GL_COLOR_BUFFER_BIT);
	for (int k = 0; k < VSU_USED; k++) {
		float w[16] = { 0.0f };

		w[k] = 1.0f;
		glUniform4fv(lw, 4, w);
		glViewport((GLint)((uint32_t)(k % 4) * VSU_CELL), (GLint)((uint32_t)(k / 4) * VSU_CELL), VSU_CELL, VSU_CELL);
		glDrawArrays(GL_TRIANGLES, 0, 6);
	}
	glFinish();
	glViewport(0, 0, TW, TH);
	if (!check_gl_error("vsunits draws")) {
		ok = false;
		goto out;
	}
	read_topdown();
	for (int k = 0; k < VSU_UNITS; k++) {
		char what[24];
		/* cell k's centre in GL window coordinates, then top-down */
		uint32_t x = (uint32_t)(k % 4) * VSU_CELL + VSU_CELL / 2u;
		uint32_t y = TH - 1u - ((uint32_t)(k / 4) * VSU_CELL + VSU_CELL / 2u);

		snprintf(what, sizeof(what), "vsunits: unit %d", k);
		ok &= expect_px(what, x, y, k < VSU_USED ? vsu_rgba(k) : VSU_BLUE, 2);
	}
out:
	glDeleteTextures(VSU_USED, tex);
	if (VSU_FS_USED > 0)
		glDeleteTextures(VSU_FS_USED, ftex);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 150: which vertex shared registers a shader reads ---------------
 *
 * Case 149 cannot tell "sh256 and up are lost" from "the whole block is
 * lost": its one-hot weights u_w follow the sampler descriptors, so at 13
 * samplers they sit at sh260.. and a loss past sh255 alone blacks out every
 * cell. This probe has no samplers. Its vertex block is one array u_p[] from
 * sh0 (vec4 i at sh4i); the one-hot selectors sit at u_p[56..58] (sh224..235,
 * below sh256), and every other u_p[i] holds shp_rgba(i), a colour that
 * names i. Draw j of an arm writes the vec4 its selector j picks into one
 * cell; the last draw of each arm writes the selector-only constant magenta,
 * which reads nothing but sh233.
 *   arm A: u_p[64], sh0..sh255 (the block that is known to work): the control.
 *   arm B: u_p[70], sh0..sh279: the same reads, plus u_p[64..69].
 * Every cell prints the index its colour names, so "reads 0" (black), "reads
 * the right vec4" and "reads another vec4" (the index says which) are three
 * different lines. Not in the default set: arm B is a probe of a draw the
 * backend refuses (sh >= 256), and needs that refusal lifted to run. */
#define SHP_SEL0 56 /* u_p[56..58]: 12 one-hot selector floats */
#define SHP_CELLS 16
#define SHP_MAX_VEC4 70
static const int shp_idx[] = { 0, 32, 60, 63, 64, 65, 66, 67, 69 };
#define SHP_N_IDX ((int)(sizeof(shp_idx) / sizeof(shp_idx[0])))
#define SHP_SEL_CONST 9 /* selector 9: the magenta constant */
#define SHP_MAGENTA 0xffff00ffu

/* u_p[i]'s colour as s_px holds it: R = 3i, G = 255 - 3i, B alternates
 * 0x40 / 0xc0 with i's parity. Every i < 85 has its own colour, none black. */
static uint32_t shp_rgba(int i)
{
	uint32_t r = 3u * (uint32_t)i, g = 255u - 3u * (uint32_t)i, b = (i & 1) ? 0xc0u : 0x40u;

	return 0xff000000u | (b << 16) | (g << 8) | r;
}

/* The index a read-back colour names, or -1 when it names none. */
static int shp_index_of(uint32_t px)
{
	for (int i = 0; i < 85; i++)
		if (chan_diff(px, shp_rgba(i)) <= 2)
			return i;
	return -1;
}

/* The probe's shader for `n_vec4`: in the vertex stage (case 150) it
 * passes the sum on in v_c; in the fragment stage (case 166) it writes it
 * to gl_FragColor. The sum is the same text either way. */
static void shp_src(char *buf, size_t size, int n_vec4, bool frag)
{
	static const char comp[4] = { 'x', 'y', 'z', 'w' };
	size_t len = (size_t)snprintf(buf, size,
				      "#version 100\n"
				      "%s"
				      "uniform vec4 u_p[%d];\n"
				      "%s"
				      "void main() {\n"
				      "    vec4 r = u_p[%d].%c * vec4(1.0, 0.0, 1.0, 1.0);\n",
				      frag ? "precision mediump float;\n" : "attribute vec4 a_pos;\n", n_vec4,
				      frag ? "" : "varying vec4 v_c;\n", SHP_SEL0 + SHP_SEL_CONST / 4,
				      comp[SHP_SEL_CONST % 4]);

	for (int j = 0; j < SHP_N_IDX && len < size; j++)
		if (shp_idx[j] < n_vec4)
			len += (size_t)snprintf(buf + len, size - len, "    r += u_p[%d].%c * u_p[%d];\n",
						SHP_SEL0 + j / 4, comp[j % 4], shp_idx[j]);
	if (len < size)
		(void)snprintf(buf + len, size - len, "%s",
			       frag ? "    gl_FragColor = r;\n}\n" : "    v_c = r;\n    gl_Position = a_pos;\n}\n");
}

/* Case 166's vertex shader: the quad, nothing else. */

/* One arm: draws into cells first_cell.. and judges them. `tag` names the
 * case in the log ("shprobe" 150, "fshprobe" 166); `frag` puts the probe
 * in the fragment stage. */
static bool shp_arm(const char *tag, bool frag, const char *arm, int n_vec4, int first_cell, int *cells_used)
{
	static char src[2048];
	static float vals[SHP_MAX_VEC4 * 4];
	struct puv_vertex q[6];
	int ncell = 0, sel[SHP_CELLS];
	GLuint prog;
	GLint lp, lu;
	bool ok = true;

	for (int j = 0; j < SHP_N_IDX; j++)
		if (shp_idx[j] < n_vec4)
			sel[ncell++] = j;
	sel[ncell++] = SHP_SEL_CONST;
	*cells_used = ncell;

	shp_src(src, sizeof(src), n_vec4, frag);
	prog = frag ? link_program(gl_test_fshp_vs_src, src) : link_program(src, gl_test_vsunits_fs_src);
	if (!prog)
		return false;
	glUseProgram(prog);
	lp = glGetAttribLocation(prog, "a_pos");
	lu = glGetUniformLocation(prog, "u_p");
	if (lp < 0 || lu < 0) {
		pr_err("%s: %s: a_pos %d u_p %d\n", tag, arm, lp, lu);
		glDeleteProgram(prog);
		return false;
	}
	pr_info("gl_test: %s: arm %s: %s u_p[%d] = sh0..sh%d\n", tag, arm, frag ? "fragment" : "vertex", n_vec4,
		4 * n_vec4 - 1);
	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glEnableVertexAttribArray((GLuint)lp);
	for (int c = 0; c < ncell; c++) {
		int cell = first_cell + c;

		for (int i = 0; i < n_vec4; i++) {
			uint32_t px = shp_rgba(i);

			for (int k = 0; k < 4; k++)
				vals[4 * i + k] = (float)((px >> (8 * k)) & 0xffu) / 255.0f;
		}
		for (int k = 0; k < 12; k++)
			vals[4 * SHP_SEL0 + k] = (k == sel[c]) ? 1.0f : 0.0f;
		glUniform4fv(lu, n_vec4, vals);
		glViewport((GLint)((uint32_t)(cell % 4) * VSU_CELL), (GLint)((uint32_t)(cell / 4) * VSU_CELL), VSU_CELL,
			   VSU_CELL);
		glDrawArrays(GL_TRIANGLES, 0, 6);
	}
	glFinish();
	glViewport(0, 0, TW, TH);
	if (!check_gl_error(frag ? "fshprobe draws" : "shprobe draws"))
		ok = false;
	read_topdown();
	for (int c = 0; c < ncell; c++) {
		int cell = first_cell + c;
		uint32_t x = (uint32_t)(cell % 4) * VSU_CELL + VSU_CELL / 2u;
		uint32_t y = TH - 1u - ((uint32_t)(cell / 4) * VSU_CELL + VSU_CELL / 2u);
		uint32_t got = s_px[y * TW + x];
		char what[40];

		if (sel[c] == SHP_SEL_CONST) {
			pr_info("gl_test: %s: arm %s cell %d: selector sh%d only -> 0x%08x (%s)\n", tag, arm, cell,
				4 * SHP_SEL0 + SHP_SEL_CONST, got,
				chan_diff(got, SHP_MAGENTA) <= 2 ? "magenta" : got == 0xff000000u ? "black" : "other");
			snprintf(what, sizeof(what), "%s %s: selector", tag, arm);
			ok &= expect_px(what, x, y, SHP_MAGENTA, 2);
		} else {
			int want = shp_idx[sel[c]], at = shp_index_of(got);

			pr_info("gl_test: %s: arm %s cell %d: u_p[%d] (sh%d..%d) -> 0x%08x = %s%d\n", tag, arm, cell,
				want, 4 * want, 4 * want + 3, got, at < 0 ? "no index " : "u_p ", at);
			snprintf(what, sizeof(what), "%s %s: u_p[%d]", tag, arm, want);
			ok &= expect_px(what, x, y, shp_rgba(want), 2);
		}
	}
	glDeleteProgram(prog);
	return ok;
}

static int case_shprobe(void)
{
	int used_a = 0, used_b = 0;
	bool ok;

	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f); /* blue: "drew nothing" is not "drew black" */
	glClear(GL_COLOR_BUFFER_BIT);
	ok = shp_arm("shprobe", false, "A", 64, 0, &used_a);
	ok &= shp_arm("shprobe", false, "B", SHP_MAX_VEC4, used_a, &used_b);
	return ok ? 0 : -1;
}

/* ---- case 166: which fragment shared registers a shader reads ------------
 *
 * The fragment twin of case 150: the same reads, made by the fragment
 * stage, where past sh255 only the host listing had checked them (idx0,
 * prepare::index_high_shared). The fragment shared layout is the vertex
 * one: the sampled textures' descriptors first from sh0, 20 dwords each
 * (image state at +0, sampler state at +8), then the uniform block at
 * sh(20 * samplers) (chain.rs expand_samples, backend.rs build_desc_block).
 * What only the fragment block has is its tail -- the discard control
 * word, gl_FragCoord's y pair, the front-facing word, blend constants --
 * which index_high_shared refuses past sh255; none is used here.
 *   arm A: u_p[64], no sampler: sh0..sh255, the control.
 *   arm B: u_p[70], no sampler: sh0..sh279; u_p[64..69] (moves) read
 *          through idx0.
 *   arm C: 16 samplers, sampler k a 1x1 texture of vsu_rgba(k) on unit k:
 *          descriptors sh0..sh319, so samplers 13, 14, 15 (sh260, sh280,
 *          sh300) are read through idx0 by the sample itself; their
 *          one-hot weights u_w[4] sit at sh320..sh335, past sh255 too.
 *          Draw k writes sampler k's colour into cell k.
 * Every cell prints what it read, as case 150's do. Not in the default set
 * (a probe, like 150). */
#define FSHP_UNITS 16

static void fshp_units_src(char *buf, size_t size)
{
	static const char comp[4] = { 'x', 'y', 'z', 'w' };
	size_t len = (size_t)snprintf(buf, size, "#version 100\nprecision mediump float;\nuniform vec4 u_w[4];\n");

	for (int k = 0; k < FSHP_UNITS && len < size; k++)
		len += (size_t)snprintf(buf + len, size - len, "uniform sampler2D u_s%d;\n", k);
	if (len < size)
		len += (size_t)snprintf(buf + len, size - len,
					"void main() {\n    vec2 c = vec2(0.5);\n    vec4 r = vec4(0.0);\n");
	for (int k = 0; k < FSHP_UNITS && len < size; k++)
		len += (size_t)snprintf(buf + len, size - len, "    r += u_w[%d].%c * texture2D(u_s%d, c);\n", k / 4,
					comp[k % 4], k);
	if (len < size)
		(void)snprintf(buf + len, size - len, "    gl_FragColor = r;\n}\n");
}

static bool fshp_arm_units(void)
{
	static char fs_src[2048];
	GLuint prog, tex[FSHP_UNITS] = { 0 };
	GLint lp, lw, maxu = 0;
	struct puv_vertex q[6];
	bool ok = true;

	glGetIntegerv(GL_MAX_TEXTURE_IMAGE_UNITS, &maxu);
	if (maxu < FSHP_UNITS) {
		pr_err("gl_test: fshprobe: arm C needs %d fragment texture units, GL_MAX_TEXTURE_IMAGE_UNITS = %d\n",
		       FSHP_UNITS, maxu);
		return false;
	}
	fshp_units_src(fs_src, sizeof(fs_src));
	prog = link_program(gl_test_fshp_vs_src, fs_src);
	if (!prog)
		return false;
	glUseProgram(prog);
	lp = glGetAttribLocation(prog, "a_pos");
	lw = glGetUniformLocation(prog, "u_w");
	if (lp < 0 || lw < 0) {
		pr_err("fshprobe: C: a_pos %d u_w %d\n", lp, lw);
		glDeleteProgram(prog);
		return false;
	}
	pr_info("gl_test: fshprobe: arm C: %d samplers = sh0..sh%d, u_w[4] = sh%d..sh%d\n", FSHP_UNITS,
		20 * FSHP_UNITS - 1, 20 * FSHP_UNITS, 20 * FSHP_UNITS + 15);
	glGenTextures(FSHP_UNITS, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	for (int k = 0; k < FSHP_UNITS; k++) {
		char name[8];
		uint32_t c = vsu_rgba(k);
		uint8_t texel[4] = { (uint8_t)c, (uint8_t)(c >> 8), (uint8_t)(c >> 16), (uint8_t)(c >> 24) };

		snprintf(name, sizeof(name), "u_s%d", k);
		glUniform1i(glGetUniformLocation(prog, name), k);
		glActiveTexture(GL_TEXTURE0 + (GLenum)k);
		glBindTexture(GL_TEXTURE_2D, tex[k]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, texel);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	}
	glActiveTexture(GL_TEXTURE0);
	if (!check_gl_error("fshprobe upload")) {
		ok = false;
		goto out;
	}
	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glEnableVertexAttribArray((GLuint)lp);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f); /* blue: "drew nothing" is not "drew black" */
	glClear(GL_COLOR_BUFFER_BIT);
	for (int k = 0; k < FSHP_UNITS; k++) {
		float w[16] = { 0.0f };

		w[k] = 1.0f;
		glUniform4fv(lw, 4, w);
		glViewport((GLint)((uint32_t)(k % 4) * VSU_CELL), (GLint)((uint32_t)(k / 4) * VSU_CELL), VSU_CELL, VSU_CELL);
		glDrawArrays(GL_TRIANGLES, 0, 6);
	}
	glFinish();
	glViewport(0, 0, TW, TH);
	if (!check_gl_error("fshprobe C draws")) {
		ok = false;
		goto out;
	}
	read_topdown();
	for (int k = 0; k < FSHP_UNITS; k++) {
		char what[32];
		uint32_t x = (uint32_t)(k % 4) * VSU_CELL + VSU_CELL / 2u;
		uint32_t y = TH - 1u - ((uint32_t)(k / 4) * VSU_CELL + VSU_CELL / 2u);
		uint32_t got = s_px[y * TW + x];
		int at = -1;

		for (int j = 0; j < FSHP_UNITS; j++)
			if (chan_diff(got, vsu_rgba(j)) <= 2)
				at = j;
		pr_info("gl_test: fshprobe: arm C cell %d: sampler %d (image sh%d, sampler sh%d) -> 0x%08x = %s%d\n", k,
			k, 20 * k, 20 * k + 8, got, at < 0 ? "no unit " : "unit ", at);
		snprintf(what, sizeof(what), "fshprobe C: sampler %d", k);
		ok &= expect_px(what, x, y, vsu_rgba(k), 2);
	}
out:
	glDeleteTextures(FSHP_UNITS, tex);
	glDeleteProgram(prog);
	return ok;
}

static int case_fshprobe(void)
{
	int used_a = 0, used_b = 0;
	bool ok;

	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	ok = shp_arm("fshprobe", true, "A", 64, 0, &used_a);
	ok &= shp_arm("fshprobe", true, "B", SHP_MAX_VEC4, used_a, &used_b);
	ok &= fshp_arm_units();
	return ok ? 0 : -1;
}

static int case_loopcarry(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint k;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_loopcarry_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	k = glGetUniformLocation(prog, "u_k");
	if (k < 0) {
		pr_err("gl_test: loopcarry u_k not found\n");
		return -1;
	}

	/* Three trips: 3 * 0.25 = 0.75, 191 in eight bits. The clear is blue so
	 * that "drew nothing" (0xffff0000) and "drew red = 0" (0xff000000) are
	 * different answers (imx8mp-board's caveat on the first run). */
	glUniform4f(k, 1.0f, 0.25f, 0.0f, 0.0f);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("loopcarry, three trips"))
		return -1;
	read_topdown();
	ok &= expect_px("loopcarry: three trips", TW / 2, TH / 2, 0xff0000bf, 2);

	/* Two trips: 0.5, 128 (127.5 rounds either way). */
	glUniform4f(k, 0.34f, 0.25f, 0.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("loopcarry, two trips"))
		return -1;
	read_topdown();
	ok &= expect_px("loopcarry: two trips", TW / 2, TH / 2, 0xff000080, 2);
	return ok ? 0 : -1;
}

/* ---- case 71: dead work around a loop that must still run ----------------
 *
 * es100/deadloop.frag computes an if-with-early-return nobody reads before
 * the loop, and a value carried round the loop that nobody reads after it.
 * The in-process back end deletes both (femtovg's stencil.frag lost 69 of 83
 * instructions this way), and every deletion renumbers what comes after it:
 * the break's target, the back edge's target and the back-edge list the ABI
 * checks backward jumps against. A target left pointing at the old index
 * runs the wrong instruction or refuses to assemble; either shows here.
 *
 * Drawn with the dead `if` on both sides (u_k.z = 1 and -1), because the
 * deleted selects are its join: 3 trips either way, 0.75 red.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_deadloop(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint k;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_deadloop_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	k = glGetUniformLocation(prog, "u_k");
	if (k < 0) {
		pr_err("gl_test: deadloop u_k not found\n");
		return -1;
	}
	for (int side = 0; side < 2; side++) {
		glUniform4f(k, 1.0f, 0.25f, side ? -1.0f : 1.0f, 0.0f);
		/* blue: "drew nothing" is not "drew red = 0" (see case 70) */
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(side ? "deadloop, z < 0" : "deadloop, z > 0"))
			return -1;
		read_topdown();
		ok &= expect_px(side ? "deadloop: three trips, z < 0" : "deadloop: three trips, z > 0",
				TW / 2, TH / 2, 0xff0000bf, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 72: integer literals are integers -------------------------------
 *
 * es100/intlit.frag computes one colour channel per shape:
 *
 *     red    for (int i = 0; i < 3; i = i + 1) r += 0.25;     -> 0.75
 *     green  int(u_in.x) == 2 ? 1.0 : 0.0                     -> side-dependent
 *     blue   ivec2(1, 2) and bvec2(1, 0), tested together      -> 1.0
 *
 * The in-process compiler lowered every integer literal as a FLOAT constant
 * until 2026-09-23, and the integer operations read the float's bits: on the
 * GC7000 the loop compiled to `cmp.lt.s32 ..., 3.0:f20` / `add.s32 ...,
 * 1.0:f20` and ran twice, and FemtoVG's `texType == 1` / `== 2` (an int
 * compare against a literal) could never be true, so the premultiply and
 * alpha-texture branches of 19 FemtoVG/Slint shaders were dead code. The
 * front end's evaluator (lower.rs int_literal_tests) measures the old
 * lowering at (0.5, 0, 0) on both sides of the compare, 0xff000080.
 *
 * Both sides of the compare are drawn and read back, for the reason case 68
 * gives: a compare that is always false passes a test that draws only the
 * false side.
 *
 * The offline arm needs a blob for intlit.frag, which the shader table does
 * not have until it is rebuilt (yakogl task 39); until then this is an
 * online case, out of the regression set like case 68.
 * ------------------------------------------------------------------------- */
static int case_intlit(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint in;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_intlit_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	in = glGetUniformLocation(prog, "u_in");
	if (in < 0) {
		pr_err("gl_test: intlit uniform u_in not found (%d)\n", in);
		return -1;
	}

	/* THE TRUE SIDE: int(2.0) == 2. Red 0.75 is 191.25, so 191 or 192. */
	glUniform4f(in, 2.0f, 0.0f, 0.0f, 0.0f);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("intlit, u_in.x = 2"))
		return -1;
	read_topdown();
	ok &= expect_px("u_in.x = 2: three passes, the compare true, the constructors right",
			TW / 2, TH / 2, 0xffffffbf, 2);
	ok &= expect_px("u_in.x = 2: and at a corner too", 1, 1, 0xffffffbf, 2);

	/* THE FALSE SIDE: int(1.0) != 2, green off, the rest unchanged. */
	glUniform4f(in, 1.0f, 0.0f, 0.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("intlit, u_in.x = 1"))
		return -1;
	read_topdown();
	ok &= expect_px("u_in.x = 1: the compare false, green off",
			TW / 2, TH / 2, 0xffff00bf, 2);
	ok &= expect_px("u_in.x = 1: and at a corner too", 1, 1, 0xffff00bf, 2);

	return ok ? 0 : -1;
}

/* ---- case 73: a branch on a lane that is not x -----------------------------
 *
 *     if (u_c.y > 0.5) discard;   ...   if (u_c.w > 0.5) discard;
 *     gl_FragColor = green;
 *
 * A discard cannot be if-converted, so each `if` is a real branch_unary, and
 * the in-process compiler fed it the condition as `.xyzw` where one lane held
 * the comparison: the other lanes took part in the test. The reference
 * compiler splats one lane (208 of 208 branches in its table). Drawn on three
 * sides with a blue clear: neither test true -> green; y true -> discarded;
 * w true -> discarded. u_c.x and u_c.z are set against each answer, so a test
 * that reads the wrong lane gets the wrong side.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_branchlane(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const struct { float c[4]; uint32_t want; const char *what; } side[3] = {
		{ { 1.0f, 0.0f, 1.0f, 0.0f }, 0xff00ff00, "neither test true: green" },
		{ { 0.0f, 1.0f, 0.0f, 0.0f }, 0xffff0000, "the y test true: discarded, the clear stays" },
		{ { 0.0f, 0.0f, 0.0f, 1.0f }, 0xffff0000, "the w test true: discarded, the clear stays" },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint c;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_branchlane_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	c = glGetUniformLocation(prog, "u_c");
	if (c < 0) {
		pr_err("gl_test: branchlane u_c not found\n");
		return -1;
	}
	for (int s = 0; s < 3; s++) {
		glUniform4f(c, side[s].c[0], side[s].c[1], side[s].c[2], side[s].c[3]);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(side[s].what))
			return -1;
		read_topdown();
		ok &= expect_px(side[s].what, TW / 2, TH / 2, side[s].want, 2);
		ok &= expect_px(side[s].what, 1, 1, side[s].want, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 74: the colour written inside an `if` ---------------------------
 *
 *     ifoutput.frag:   if (u_c.x > 0.5) gl_FragColor = red; else gl_FragColor = green;
 *     ifoutput2.frag:  gl_FragColor = green; if (u_c.x > 0.5) gl_FragColor = red;
 *
 * Neither reads gl_FragColor, so it is an output and not a local slot, and
 * the in-process compiler's if-conversion -- which runs both arms and joins
 * SLOTS with a select -- ran both stores one after the other: green for the
 * first program and red for the second, whatever u_c.x said. Each program is
 * drawn on both sides of the condition with a blue clear, so "drew nothing"
 * is its own answer.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_ifoutput(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	const char *const fs[2] = { gl_test_ifoutput_fs_src, gl_test_ifoutput2_fs_src };
	const char *const what[2] = { "ifoutput if/else", "ifoutput if-only" };
	struct pc_vertex v[6];
	bool ok = true;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	for (int p = 0; p < 2; p++) {
		GLuint prog = link_program(gl_test_passthrough_vs_src, fs[p]);
		GLint c;

		if (!prog)
			return -1;
		glUseProgram(prog);
		upload_vbo(v, sizeof(v));
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		c = glGetUniformLocation(prog, "u_c");
		if (c < 0) {
			pr_err("gl_test: %s: u_c not found\n", what[p]);
			return -1;
		}
		for (int side = 0; side < 2; side++) {
			/* true side red, false side green, in both programs */
			glUniform4f(c, side ? 0.0f : 1.0f, 0.0f, 0.0f, 0.0f);
			glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
			if (!check_gl_error(what[p]))
				return -1;
			read_topdown();
			ok &= expect_px(side ? "false side: green" : "true side: red", TW / 2, TH / 2,
					side ? 0xff00ff00 : 0xff0000ff, 2);
		}
	}
	return ok ? 0 : -1;
}

#ifdef GL_TEST_V3D
/* ---- case 109: LINEAR filtering against dEQP's reference ------------------
 *
 * Slint samples its text and images with LINEAR, and nothing on the host can
 * say what the TMU returns for it (docs/online-shaderc/v3d-backend.md, "LINEAR
 * on the board"). The scene, the four samplers, the reference (dEQP's
 * sampleLinear2D, tools/v3d/tex_filter_ref.h) and the verdict against dEQP's
 * GLES2 bound are tools/v3d/tex_linear_case.h, which
 * tools/v3d/tex_filter_ref_check.c checks on the host; this case only draws
 * and reads back. A quad of size x size pixels at the lower-left corner, its
 * texture coordinates 0..reps, one draw per sampler. V3D only for now: the
 * header is on v3d_gl_test's include path only. */
#include "tex_linear_case.h"

static int case_texlinear(void)
{
	GLuint prog, tex = 0;
	bool ok = true;
	const uint8_t *texels = tlc_make_texture();

	if (!tlc_controls(printk, NULL))
		return -1;
	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	/* row 0 of the upload is t = 0, as the header's reference assumes */
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TLC_TEX_W, TLC_TEX_H, 0, GL_RGBA,
		     GL_UNSIGNED_BYTE, texels);
	if (!check_gl_error("texlinear upload"))
		return -1;

	for (int i = 0; i < TLC_DRAWS; i++) {
		const struct tlc_draw *d = &tlc_draws[i];
		const GLint wrap = d->wrap == TFR_REPEAT ? GL_REPEAT : GL_CLAMP_TO_EDGE;
		struct puv_vertex q[6];
		struct tlc_result r;

		puv_quad(q);
		for (int v = 0; v < 6; v++) {
			q[v].uv[0] *= d->reps;
			q[v].uv[1] *= d->reps;
		}
		upload_vbo(q, sizeof(q));
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, d->min_linear ? GL_LINEAR : GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, d->mag_linear ? GL_LINEAR : GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, wrap);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap);
		glViewport(0, 0, d->size, d->size);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("texlinear draw")) {
			ok = false;
			break;
		}
		read_topdown();
		/* pixel (x, y) in GL window coordinates, from the lower-left
		 * corner; s_px is top-down, and its bytes are R, G, B, A */
		tlc_result_init(&r);
		for (int y = 0; y < d->size; y++)
			for (int x = 0; x < d->size; x++) {
				uint8_t px[4];

				memcpy(px, &s_px[(TH - 1u - (uint32_t)y) * TW + (uint32_t)x], 4);
				tlc_check_pixel(d, x, y, px, &r);
			}
		ok &= gl_test_check(tlc_verdict(d, &r, printk), (uint32_t)r.over, 0);
	}
	glViewport(0, 0, TW, TH);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 110: the point-size word on a triangle and a line ----------------
 *
 * Mesa's V3D driver puts the point size into the VS's VPM output only for a
 * points draw, and sets the shader record's "Point size in shaded vertex
 * data" only then (v3dx_draw.c:556-558). Our backend sets the bit from the
 * program (the VS writes the word: writes_psiz). If the bit only describes
 * the VPM layout whatever the primitive, one program serves points and
 * triangles alike and no points variant is needed; if the hardware reads the
 * word only for points, a triangle drawn with it reads its varyings one word
 * off. Nothing we have read answers it, so the board does:
 * the same triangle and the same line, drawn by a VS that also writes
 * gl_PointSize and by the same VS without it, compared over the whole
 * framebuffer. v3d_psiz_draws() must grow by exactly the two draws with the
 * word, or the case says NOT EXERCISED: a compiler that drops gl_PointSize
 * (every build until the switch) draws two identical images and proves
 * nothing. */
extern uint32_t v3d_psiz_draws(void);



/* One draw of `n` vertices of `mode` with `prog`, read back into `out`. */
static bool psizvpm_draw(GLuint prog, GLenum mode, const float (*pos)[4], const float (*col)[4],
			 GLsizei n, uint32_t *out)
{
	GLint lp = glGetAttribLocation(prog, "a_pos"), lc = glGetAttribLocation(prog, "a_color");

	if (lp < 0 || lc < 0) {
		pr_err("gl_test: psizvpm: attribute locations %d, %d\n", lp, lc);
		return false;
	}
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, 0, pos);
	glVertexAttribPointer((GLuint)lc, 4, GL_FLOAT, GL_FALSE, 0, col);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lc);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(mode, 0, n);
	glFinish();
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lc);
	if (!check_gl_error("psizvpm draw"))
		return false;
	read_topdown();
	memcpy(out, s_px, sizeof(s_px));
	return true;
}

static int case_psizvpm(void)
{
	/* a triangle with a colour gradient on every edge, and three lines */
	static const float tri_pos[3][4] = {
		{ -0.9f, -0.9f, 0.0f, 1.0f }, { 0.9f, -0.7f, 0.0f, 1.0f }, { -0.2f, 0.9f, 0.0f, 1.0f },
	};
	static const float tri_col[3][4] = {
		{ 1.0f, 0.0f, 0.0f, 1.0f }, { 0.0f, 1.0f, 0.0f, 1.0f }, { 0.0f, 0.0f, 1.0f, 1.0f },
	};
	static const float line_pos[6][4] = {
		{ -0.9f, -0.5f, 0.0f, 1.0f }, { 0.9f, -0.4f, 0.0f, 1.0f },
		{ -0.8f, 0.8f, 0.0f, 1.0f },  { 0.7f, -0.9f, 0.0f, 1.0f },
		{ 0.1f, -0.9f, 0.0f, 1.0f },  { 0.2f, 0.9f, 0.0f, 1.0f },
	};
	static const float line_col[6][4] = {
		{ 1.0f, 0.0f, 0.0f, 1.0f }, { 0.0f, 0.0f, 1.0f, 1.0f },
		{ 0.0f, 1.0f, 0.0f, 1.0f }, { 1.0f, 1.0f, 0.0f, 1.0f },
		{ 0.0f, 1.0f, 1.0f, 1.0f }, { 1.0f, 0.0f, 1.0f, 1.0f },
	};
	static uint32_t plain[TW * TH], word[TW * TH];
	const struct { const char *name; GLenum mode; const float (*pos)[4], (*col)[4]; GLsizei n;
		       uint32_t min_lit; } draws[2] = {
		{ "triangle", GL_TRIANGLES, tri_pos, tri_col, 3, 800 },
		{ "lines", GL_LINES, line_pos, line_col, 6, 100 },
	};
	GLuint pp = link_program(gl_test_psizvpm_plain_vs_src, gl_test_interp_fs_src);
	GLuint pw = link_program(gl_test_psizvpm_word_vs_src, gl_test_interp_fs_src);
	bool ok = true;

	if (!pp || !pw)
		return -1;
	for (int i = 0; i < 2; i++) {
		uint32_t before, counted, lit = 0, differ = 0, first = TW * TH;

		if (!psizvpm_draw(pp, draws[i].mode, draws[i].pos, draws[i].col, draws[i].n, plain))
			return -1;
		before = v3d_psiz_draws();
		if (!psizvpm_draw(pw, draws[i].mode, draws[i].pos, draws[i].col, draws[i].n, word))
			return -1;
		counted = v3d_psiz_draws() - before;
		for (uint32_t k = 0; k < TW * TH; k++) {
			lit += (plain[k] & 0xffffffu) != 0;
			if (plain[k] != word[k]) {
				differ++;
				if (first == TW * TH)
					first = k;
			}
		}
		printk("gl_test: psizvpm %s: %u lit pixels, %u draw(s) with the point size in the VPM, "
		       "%u of %u pixels differ\n",
		       draws[i].name, lit, counted, differ, TW * TH);
		if (differ)
			printk("gl_test: psizvpm %s: first difference at (%u, %u) from the top: "
			       "0x%08x without the word, 0x%08x with it\n",
			       draws[i].name, first % TW, first / TW, plain[first], word[first]);
		/* the known positive: the plain draw drew */
		ok &= gl_test_check(lit >= draws[i].min_lit, lit, draws[i].min_lit);
		if (counted != 1) {
			printk("gl_test: psizvpm %s: NOT EXERCISED (%u draws carried the word, 1 expected): "
			       "the compiler dropped gl_PointSize, so the images say nothing\n",
			       draws[i].name, counted);
			ok &= gl_test_check(false, counted, 1);
			continue;
		}
		ok &= gl_test_check(differ == 0, differ, 0);
	}
	glUseProgram(0);
	glDeleteProgram(pp);
	glDeleteProgram(pw);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 111: GL_POINTS on the window and on an FBO -------------------------
 *
 * The V3D draws points with the triangles program: the VS keeps the
 * gl_PointSize word in its VPM output for every primitive, and the shader
 * record says so (case 110 measured that the bit only describes the layout).
 * A VS that does not write gl_PointSize leaves the bit off, and the
 * POINT_SIZE packet's 1.0 is the size (ES 2.0 leaves it undefined; this pins
 * what we give).
 *
 * Three points at pixel centres, asymmetric so a y flip shows: sizes 1, 3 and
 * 5 from gl_PointSize (a square of s x s pixels centred on the pixel, GL ES
 * 2.0 3.3), red, green and blue; then the same points from a VS without
 * gl_PointSize, one pixel each. Every pixel of the 64x64 frame is checked
 * against that picture, on the window and on an RGBA texture FBO.
 * v3d_psiz_draws() must grow across the gl_PointSize draws, or the case says
 * NOT EXERCISED (the size word never reached a job). */
static const uint32_t points_at[3][2] = { { 8, 8 }, { 40, 24 }, { 50, 52 } };
static const uint32_t points_rgba[3] = { 0xff0000ffu, 0xff00ff00u, 0xffff0000u }; /* AABBGGRR */

/* The frame's colour at (x, y) (top-down) for points of `sizes`. */
static uint32_t points_want(uint32_t x, uint32_t y, const uint32_t sizes[3])
{
	for (int i = 0; i < 3; i++) {
		uint32_t h = sizes[i] / 2;

		if (x + h >= points_at[i][0] && x <= points_at[i][0] + h && y + h >= points_at[i][1] &&
		    y <= points_at[i][1] + h)
			return points_rgba[i];
	}
	return 0xff000000u;
}

/* One frame: clear black, draw the three points with `prog`, check it all. */
static bool points_frame(const char *what, GLuint prog, const uint32_t sizes[3])
{
	struct pv { float pos[4]; float color[4]; float size; } pv[3];
	GLint lp = glGetAttribLocation(prog, "a_pos"), lc = glGetAttribLocation(prog, "a_color");
	GLint ls = glGetAttribLocation(prog, "a_size");
	uint32_t bad = 0, first = TW * TH;

	if (lp < 0 || lc < 0) {
		pr_err("gl_test: points: %s: attribute locations %d, %d\n", what, lp, lc);
		return false;
	}
	for (int i = 0; i < 3; i++) {
		pv[i].pos[0] = ndc_x(points_at[i][0]);
		pv[i].pos[1] = ndc_y(points_at[i][1]);
		pv[i].pos[2] = 0.0f;
		pv[i].pos[3] = 1.0f;
		for (int c = 0; c < 4; c++)
			pv[i].color[c] = (float)((points_rgba[i] >> (8 * c)) & 0xff) / 255.0f;
		pv[i].size = (float)sizes[i];
	}
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv[0].pos);
	glVertexAttribPointer((GLuint)lc, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv[0].color);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lc);
	if (ls >= 0) {
		glVertexAttribPointer((GLuint)ls, 1, GL_FLOAT, GL_FALSE, sizeof(struct pv), &pv[0].size);
		glEnableVertexAttribArray((GLuint)ls);
	}
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_POINTS, 0, 3);
	glFinish();
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lc);
	if (ls >= 0)
		glDisableVertexAttribArray((GLuint)ls);
	if (!check_gl_error(what))
		return false;
	read_topdown();
	for (uint32_t y = 0; y < TH; y++)
		for (uint32_t x = 0; x < TW; x++)
			if (chan_diff(s_px[y * TW + x], points_want(x, y, sizes)) > 1) {
				bad++;
				if (first == TW * TH)
					first = y * TW + x;
			}
	printk("gl_test: points %s: %u of %u pixels differ from the picture\n", what, bad, TW * TH);
	if (bad)
		printk("gl_test: points %s: first at (%u, %u) from the top: 0x%08x, want 0x%08x\n", what,
		       first % TW, first / TW, s_px[first], points_want(first % TW, first / TW, sizes));
	return gl_test_check(bad == 0, bad, 0);
}

static int case_points(void)
{
	static const uint32_t sized[3] = { 1, 3, 5 }, one[3] = { 1, 1, 1 };
	GLuint psz = link_program(gl_test_points_vs_src, gl_test_interp_fs_src);
	GLuint plain = link_program(gl_test_psizvpm_plain_vs_src, gl_test_interp_fs_src);
	GLuint tex = 0, fbo = 0;
	uint32_t before;
	bool ok = true;

	if (!psz || !plain)
		return -1;
	for (int target = 0; target < 2; target++) {
		const char *where = target ? "fbo" : "window";
		char what[40];

		if (target) {
			glGenTextures(1, &tex);
			glBindTexture(GL_TEXTURE_2D, tex);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
			glGenFramebuffers(1, &fbo);
			glBindFramebuffer(GL_FRAMEBUFFER, fbo);
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
		}
		before = v3d_psiz_draws();
		snprintf(what, sizeof(what), "%s, gl_PointSize 1/3/5", where);
		ok &= points_frame(what, psz, sized);
		if (v3d_psiz_draws() == before) {
			printk("gl_test: points %s: NOT EXERCISED: no draw carried the point size "
			       "(the compiler dropped gl_PointSize)\n", where);
			ok &= gl_test_check(false, 0, 1);
		}
		snprintf(what, sizeof(what), "%s, no gl_PointSize", where);
		ok &= points_frame(what, plain, one);
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(psz);
	glDeleteProgram(plain);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 136: where the V3D clamps gl_PointSize ----------------------------
 *
 * dEQP rasterization.limits.points (FAIL on the Pi 4, c9eb1078,
 * es2fRasterizationTests.cpp:686) draws a point of GL_ALIASED_POINT_SIZE_RANGE's
 * maximum + 16 and expects it drawn at the maximum. The GL core reports 64
 * for every GPU (src/gl/state.rs), and nothing on the V3D clamps to 64. Mesa
 * reports V3D_MAX_POINT_SIZE = 512 (broadcom/common/v3d_limits.h:46) and adds
 * no upper clamp in the shader (vir.c:1112, nir_lower_point_size(1.0, 0.0)),
 * so the hardware clamps, at or below 512. This measures where.
 *
 * One red point per size in a PSM_W x PSM_H RGBA texture FBO, centred on a
 * pixel edge for an even size and on a pixel centre for an odd one, so a
 * square of side s covers exactly s fragment centres per row (GL ES 2.0
 * 3.3). The width is the red run on row PSM_ROW, which the square covers
 * for every size here. The verdict is the property a [1, 512] range needs:
 * width s for s <= 512 and 512 above it; every width is printed either way,
 * with the largest drawn ("clamps at"). v3d_psiz_draws() must grow, or the
 * case says NOT EXERCISED. V3D only. */
#define PSM_W   1280
#define PSM_H   4
#define PSM_ROW 2
static uint32_t psm_row[PSM_W];

static int case_psizmax(void)
{
	static const uint32_t sizes[] = { 64, 80, 256, 511, 512, 513, 600, 1024 };
	struct pv { float pos[4]; float color[4]; float size; } pv;
	GLuint prog = link_program(gl_test_points_vs_src, gl_test_interp_fs_src);
	GLuint tex = 0, fbo = 0;
	GLint lp, lc, ls;
	uint32_t before, widest = 0;
	bool ok = true;

	if (!prog)
		return -1;
	lp = glGetAttribLocation(prog, "a_pos");
	lc = glGetAttribLocation(prog, "a_color");
	ls = glGetAttribLocation(prog, "a_size");
	if (lp < 0 || lc < 0 || ls < 0) {
		pr_err("gl_test: psizmax: attribute locations %d, %d, %d\n", lp, lc, ls);
		glDeleteProgram(prog);
		return -1;
	}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, PSM_W, PSM_H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: psizmax: the %ux%u FBO is not complete\n", PSM_W, PSM_H);
		ok = false;
		goto out;
	}
	glViewport(0, 0, PSM_W, PSM_H);
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(pv), pv.pos);
	glVertexAttribPointer((GLuint)lc, 4, GL_FLOAT, GL_FALSE, sizeof(pv), pv.color);
	glVertexAttribPointer((GLuint)ls, 1, GL_FLOAT, GL_FALSE, sizeof(pv), &pv.size);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lc);
	glEnableVertexAttribArray((GLuint)ls);
	before = v3d_psiz_draws();
	for (unsigned i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		const uint32_t s = sizes[i], want = s <= 512 ? s : 512;
		/* window coordinates of the centre: a pixel edge for an even size */
		const float xw = PSM_W / 2 + (s & 1 ? 0.5f : 0.0f), yw = PSM_ROW + 0.5f;
		uint32_t n = 0, lo = PSM_W, hi = 0;

		pv.pos[0] = 2.0f * xw / PSM_W - 1.0f;
		pv.pos[1] = 2.0f * yw / PSM_H - 1.0f;
		pv.pos[2] = 0.0f;
		pv.pos[3] = 1.0f;
		pv.color[0] = 1.0f;
		pv.color[1] = pv.color[2] = 0.0f;
		pv.color[3] = 1.0f;
		pv.size = (float)s;
		glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_POINTS, 0, 1);
		glReadPixels(0, PSM_ROW, PSM_W, 1, GL_RGBA, GL_UNSIGNED_BYTE, psm_row);
		if (!check_gl_error("psizmax")) {
			ok = false;
			break;
		}
		for (uint32_t x = 0; x < PSM_W; x++)
			if (chan_diff(psm_row[x], 0xff0000ffu) <= 1) {
				n++;
				lo = x < lo ? x : lo;
				hi = x;
			}
		/* one run, centred: every red pixel between lo and hi */
		const bool one_run = n == 0 || hi - lo + 1 == n;
		printk("gl_test: psizmax: gl_PointSize %u: %u pixels wide (x %u..%u%s), want %u\n", s, n,
		       n ? lo : 0, n ? hi : 0, one_run ? "" : ", NOT one run", want);
		widest = n > widest ? n : widest;
		ok &= gl_test_check(n == want && one_run, n, want);
	}
	printk("gl_test: psizmax: the widest point drawn: %u pixels (clamps at %u or above)\n", widest, widest);
	if (v3d_psiz_draws() == before) {
		printk("gl_test: psizmax: NOT EXERCISED: no draw carried the point size\n");
		ok &= gl_test_check(false, 0, 1);
	}
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lc);
	glDisableVertexAttribArray((GLuint)ls);
out:
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 137: gl_PointSize below 1.0 on the V3D -------------------------------
 *
 * GL ES 2.0 3.3: the point size is clamped to ALIASED_POINT_SIZE_RANGE,
 * [1, 512] on the V3D, so 0 < gl_PointSize < 1 must draw a 1-pixel point.
 * Mesa commit 3539bd63 (MR !1499, 2019): "the hardware won't clamp to the
 * minimum side of the range and won't render points with a size strictly
 * smaller than 1.0 either, so we need to clamp manually" (vir.c:1112,
 * nir_lower_point_size(1.0, 0.0)). Our online V3D compiler has no lower
 * clamp. This measures what the board draws.
 *
 * One red point per size (0.25, 0.5, 0.99, 1.0, 1.5) at the centre of pixel
 * (PMN_AT, PMN_AT) of a PMN_W x PMN_W RGBA texture FBO cleared green. A
 * point of side 1.5 centred on a pixel centre covers that one pixel centre
 * too. Every size must draw exactly that one pixel red and nothing else;
 * each size prints its red pixel count and their box, or NOTHING drawn.
 * v3d_psiz_draws() must grow, or NOT EXERCISED. V3D only. */
#define PMN_W  16
#define PMN_AT 8
static uint32_t pmn_px[PMN_W * PMN_W];

static int case_psizmin(void)
{
	static const float sizes[] = { 0.25f, 0.5f, 0.99f, 1.0f, 1.5f };
	static const char *const names[] = { "0.25", "0.5", "0.99", "1.0", "1.5" };
	struct pv { float pos[4]; float color[4]; float size; } pv;
	GLuint prog = link_program(gl_test_points_vs_src, gl_test_interp_fs_src);
	GLuint tex = 0, fbo = 0;
	GLint lp, lc, ls;
	uint32_t before;
	bool ok = true;

	if (!prog)
		return -1;
	lp = glGetAttribLocation(prog, "a_pos");
	lc = glGetAttribLocation(prog, "a_color");
	ls = glGetAttribLocation(prog, "a_size");
	if (lp < 0 || lc < 0 || ls < 0) {
		pr_err("gl_test: psizmin: attribute locations %d, %d, %d\n", lp, lc, ls);
		glDeleteProgram(prog);
		return -1;
	}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, PMN_W, PMN_W, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: psizmin: the %ux%u FBO is not complete\n", PMN_W, PMN_W);
		ok = false;
		goto out;
	}
	glViewport(0, 0, PMN_W, PMN_W);
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(pv), pv.pos);
	glVertexAttribPointer((GLuint)lc, 4, GL_FLOAT, GL_FALSE, sizeof(pv), pv.color);
	glVertexAttribPointer((GLuint)ls, 1, GL_FLOAT, GL_FALSE, sizeof(pv), &pv.size);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lc);
	glEnableVertexAttribArray((GLuint)ls);
	before = v3d_psiz_draws();
	for (unsigned i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		uint32_t n = 0, x0 = PMN_W, x1 = 0, y0 = PMN_W, y1 = 0;
		bool at = false;

		/* the centre of pixel (PMN_AT, PMN_AT) */
		pv.pos[0] = 2.0f * (PMN_AT + 0.5f) / PMN_W - 1.0f;
		pv.pos[1] = pv.pos[0];
		pv.pos[2] = 0.0f;
		pv.pos[3] = 1.0f;
		pv.color[0] = 1.0f;
		pv.color[1] = pv.color[2] = 0.0f;
		pv.color[3] = 1.0f;
		pv.size = sizes[i];
		glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_POINTS, 0, 1);
		glReadPixels(0, 0, PMN_W, PMN_W, GL_RGBA, GL_UNSIGNED_BYTE, pmn_px);
		if (!check_gl_error("psizmin")) {
			ok = false;
			break;
		}
		/* rows bottom-up, as glReadPixels returns them */
		for (uint32_t y = 0; y < PMN_W; y++)
			for (uint32_t x = 0; x < PMN_W; x++)
				if (chan_diff(pmn_px[y * PMN_W + x], 0xff0000ffu) <= 1) {
					n++;
					x0 = x < x0 ? x : x0;
					x1 = x > x1 ? x : x1;
					y0 = y < y0 ? y : y0;
					y1 = y > y1 ? y : y1;
					at |= x == PMN_AT && y == PMN_AT;
				}
		if (n)
			printk("gl_test: psizmin: gl_PointSize %s: %u red pixels (x %u..%u, y %u..%u)%s\n", names[i],
			       n, x0, x1, y0, y1, at ? "" : ", NOT at the point's pixel");
		else
			printk("gl_test: psizmin: gl_PointSize %s: NOTHING drawn\n", names[i]);
		ok &= gl_test_check(n == 1 && at, n, 1);
	}
	if (v3d_psiz_draws() == before) {
		printk("gl_test: psizmin: NOT EXERCISED: no draw carried the point size\n");
		ok &= gl_test_check(false, 0, 1);
	}
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lc);
	glDisableVertexAttribArray((GLuint)ls);
out:
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 140: the TFU against our CPU tiling (tfuprobe) --------------------
 *
 * The measurement behind a decision (the TFU note, 2026-09-26): no product
 * code submits a TFU job. Opt-in: in no default list.
 *
 * Part 1, bytes. For every format of tools/v3d/tfu_case.h (RGBA8 as R32F
 * and as RGBA8, RG8 as R16F and as RG8, R8), every size (odd widths reaching
 * LT, UBLINEAR 1 and 2, UIF, UIF XOR with OPAD 5) and every source variant
 * (stride rounded to 64 bytes or tight; the source at +0, +64 or +4 from a
 * page), a patterned raster source in V3D memory is tiled by one TFU job
 * (osal_v3d_tfu, polled) into an image filled with 0xee, and compared at
 * every texel with v3d_tex_store_level's image of the same source. One line
 * per format and size: each variant "ok", "W<wrong texels>" or "E<errno>",
 * and the padding bytes the TFU wrote ("pad+N": a finding; the TMU never
 * reads padding). The verdict is Mesa's shape (s64+0) on every format and
 * size: every job 0 and every texel equal. The other variants are findings.
 *
 * Part 2, time. 1920x720 RGBA8 (the cluster's backdrop): the TFU job (as
 * RGBA8 and as R32F, 5 runs each, byte-compared once each) against the CPU
 * tiling from V3D memory (the draw path: Normal non-cacheable, 3 runs) and
 * from a cacheable buffer (the upload path, 3 runs), best and worst in us.
 *
 * Every line names the GPU resets since the case began (a TFU job that
 * faults or does not end resets the GPU; such a run is never a clean PASS).
 * V3D only. */
#include "tfu_case.h"
#include "v3d.h"
#include "zephyr_v3d.h"

/* the largest part-1 image and source (95x419 at 4 bytes: 160,896 bytes of
 * source at the 64-byte stride; the layout is under 256 KiB) */
#define TFU_P1_BYTES (256u * 1024u)
#define TFU_P2_W 1920u
#define TFU_P2_H 720u
static uint8_t tfu_ref[TFU_P1_BYTES] __aligned(64);
static uint8_t tfu_cached_src[TFU_P2_W * TFU_P2_H * 4u] __aligned(64) __noinit;

/* V3D memory of `bytes` at a page boundary (the OSAL's granule is smaller) */
struct tfu_buf {
	struct osal_gpu_mem mem;
	uint8_t *cpu;
	uint32_t gpu;
};

static bool tfu_buf_alloc(struct tfu_buf *b, size_t bytes)
{
	memset(b, 0, sizeof(*b));
	if (osal_gpu_alloc(yakogl_heap_base(GLES_HEAP_DATA), bytes + 4096u, OSAL_MEM_DATA, &b->mem) != 0 ||
	    !b->mem.cpu || b->mem.gpu > UINT32_MAX - bytes - 4096u) {
		pr_err("gl_test: tfuprobe: no V3D memory for %u bytes\n", (unsigned)bytes);
		return false;
	}
	const uint32_t pad = (uint32_t)((4096u - (b->mem.gpu & 4095u)) & 4095u);
	b->gpu = (uint32_t)b->mem.gpu + pad;
	b->cpu = (uint8_t *)b->mem.cpu + pad;
	return true;
}

static void tfu_buf_free(struct tfu_buf *b)
{
	if (b->mem.impl)
		g_osal->gpu_free(&b->mem);
	memset(b, 0, sizeof(*b));
}

static void tfu_flush(void *p, size_t n)
{
	if (g_osal->cache_flush)
		g_osal->cache_flush(p, n);
}

static void tfu_invd(void *p, size_t n)
{
	if (g_osal->cache_invd)
		g_osal->cache_invd(p, n);
}

/* one job from a packed tfu_case_job */
static int tfu_run(const struct tfu_case_job *c, struct v3d_tfu_status *st)
{
	const struct v3d_tfu_job j = {
		.icfg = c->icfg, .iia = c->iia, .iis = c->iis, .ica = c->ica,
		.iua = c->iua, .ioa = c->ioa, .ios = c->ios, .coef0 = c->coef0,
	};
	return osal_v3d_tfu(&j, st);
}

static uint64_t tfu_us(uint64_t cycles)
{
	return cycles * 1000000u / (uint64_t)sys_clock_hw_cycles_per_sec();
}

static int case_tfuprobe(void)
{
	const uint32_t rec0 = osal_v3d_recoveries();
	struct tfu_buf src = { 0 }, dst = { 0 }, big_src = { 0 }, big_tfu = { 0 }, big_cpu = { 0 };
	bool ok = true;
	uint32_t jobs = 0, verdict_jobs = 0, verdict_ok = 0;

	printk("gl_test: tfuprobe: HUB_IDENT1 says TFU %s\n", "(checked by osal_v3d_tfu)");
	if (!tfu_buf_alloc(&src, TFU_P1_BYTES) || !tfu_buf_alloc(&dst, TFU_P1_BYTES)) {
		ok = false;
		goto out;
	}
	/* part 1 */
	for (int f = 0; f < TFU_CASE_FMTS && ok; f++)
		for (int i = 0; i < TFU_CASE_SIZES; i++) {
			const struct tfu_case_fmt *fm = &tfu_case_fmts[f];
			const uint32_t w = tfu_case_sizes[i][0], h = tfu_case_sizes[i][1];
			struct v3d_tex_layout l;
			char line[200];
			int n;
			uint32_t pad_written = 0;

			if (!v3d_tex_layout(w, h, 1, fm->cpp, &l) || l.size > TFU_P1_BYTES) {
				printk("gl_test: tfuprobe: %s %ux%u: no layout\n", fm->name, w, h);
				ok = false;
				continue;
			}
			n = snprintk(line, sizeof(line), "%s %ux%u %s opad %u:", fm->name, w, h,
				     tfu_case_tiling_name(l.slices[0].tiling), l.slices[0].ub_pad);
			for (int v = 0; v < TFU_CASE_VARIANTS; v++) {
				const struct tfu_case_variant *va = &tfu_case_variants[v];
				const uint32_t stride = tfu_case_stride(w, fm->cpp, va->aligned_stride);
				uint8_t *s = src.cpu + va->src_offset;
				struct tfu_case_job job;
				struct v3d_tfu_status st;
				struct tfu_case_cmp c;

				/* the source: bytes between rows are 0x5a, never a texel */
				memset(src.cpu, 0x5a, (size_t)stride * h + va->src_offset);
				tfu_case_fill_src(s, w, h, fm->cpp, stride, jobs + 1u);
				tfu_flush(src.cpu, (size_t)stride * h + va->src_offset);
				memset(dst.cpu, TFU_CASE_FILL, l.size);
				tfu_flush(dst.cpu, l.size);
				memset(tfu_ref, TFU_CASE_FILL, l.size);
				v3d_tex_store_level(tfu_ref, &l, 0, s, stride);
				if (!tfu_pack(&l, fm->ttype, src.gpu + va->src_offset, stride, dst.gpu, &job)) {
					n += snprintk(line + n, sizeof(line) - n, " %s:refused", va->name);
					ok &= v != 0;
					continue;
				}
				int rc = tfu_run(&job, &st);
				jobs++;
				if (rc == -ENOTSUP) {
					printk("gl_test: tfuprobe: NOT EXERCISED: HUB_IDENT1 says this V3D has no TFU\n");
					ok = false;
					goto out;
				}
				tfu_invd(dst.cpu, l.size);
				tfu_case_compare(tfu_ref, dst.cpu, &l, &c);
				pad_written += c.pad_written;
				if (rc)
					n += snprintk(line + n, sizeof(line) - n, " %s:E%d", va->name, rc);
				else if (c.texel_wrong)
					n += snprintk(line + n, sizeof(line) - n, " %s:W%u(u%u,@%u,%u)", va->name,
						      c.texel_wrong, c.texel_unwritten, c.first_x, c.first_y);
				else
					n += snprintk(line + n, sizeof(line) - n, " %s:ok", va->name);
				if (v == 0) {
					verdict_jobs++;
					verdict_ok += rc == 0 && c.texel_wrong == 0 && c.texels == w * h;
				}
			}
			printk("gl_test: tfuprobe: %s; pad+%u; resets %u\n", line, pad_written,
			       osal_v3d_recoveries() - rec0);
		}
	printk("gl_test: tfuprobe: part 1: %u jobs, Mesa's shape (s64+0) %u of %u equal (planned %u)\n", jobs,
	       verdict_ok, verdict_jobs, (unsigned)(TFU_CASE_FMTS * TFU_CASE_SIZES));
	ok &= gl_test_check(verdict_jobs == TFU_CASE_FMTS * TFU_CASE_SIZES && verdict_ok == verdict_jobs,
			    verdict_ok, TFU_CASE_FMTS * TFU_CASE_SIZES);
	tfu_buf_free(&src);
	tfu_buf_free(&dst);

	/* part 2 */
	{
		struct v3d_tex_layout l;
		const uint32_t w = TFU_P2_W, h = TFU_P2_H, stride = w * 4u;
		const size_t src_bytes = (size_t)stride * h;

		if (!v3d_tex_layout(w, h, 1, 4, &l) || !tfu_buf_alloc(&big_src, src_bytes) ||
		    !tfu_buf_alloc(&big_tfu, l.size) || !tfu_buf_alloc(&big_cpu, l.size)) {
			ok = false;
			goto out;
		}
		tfu_case_fill_src(tfu_cached_src, w, h, 4, stride, 0xb16u);
		memcpy(big_src.cpu, tfu_cached_src, src_bytes);
		tfu_flush(big_src.cpu, src_bytes);

		/* the CPU, from V3D memory (the draw path) and from cacheable
		 * memory (the upload path); the last run is the reference */
		uint64_t best[2] = { UINT64_MAX, UINT64_MAX }, worst[2] = { 0, 0 };
		for (int from = 0; from < 2; from++)
			for (int r = 0; r < 3; r++) {
				memset(big_cpu.cpu, TFU_CASE_FILL, l.size);
				const uint64_t c0 = k_cycle_get_64();
				v3d_tex_store_level(big_cpu.cpu, &l, 0, from ? tfu_cached_src : big_src.cpu, stride);
				const uint64_t t = k_cycle_get_64() - c0;
				best[from] = MIN(best[from], t);
				worst[from] = MAX(worst[from], t);
			}
		printk("gl_test: tfuprobe: 1920x720 RGBA8 %s, %u bytes: CPU from V3D memory best %llu worst %llu us; "
		       "CPU from cacheable best %llu worst %llu us\n",
		       tfu_case_tiling_name(l.slices[0].tiling), l.size, (unsigned long long)tfu_us(best[0]),
		       (unsigned long long)tfu_us(worst[0]), (unsigned long long)tfu_us(best[1]),
		       (unsigned long long)tfu_us(worst[1]));

		static const uint32_t types[2] = { TFU_TTYPE_RGBA8, TFU_TTYPE_R32F };
		static const char *const tnames[2] = { "RGBA8", "R32F" };
		for (int t = 0; t < 2; t++) {
			struct tfu_case_job job;
			struct v3d_tfu_status st;
			struct tfu_case_cmp c = { 0 };
			uint64_t tb = UINT64_MAX, tw = 0;
			uint32_t poll_best = UINT32_MAX;
			int rc = 0;

			if (!tfu_pack(&l, types[t], big_src.gpu, stride, big_tfu.gpu, &job)) {
				printk("gl_test: tfuprobe: 1920x720 as %s: refused by tfu_pack\n", tnames[t]);
				ok = false;
				continue;
			}
			for (int r = 0; r < 5 && rc == 0; r++) {
				memset(big_tfu.cpu, TFU_CASE_FILL, l.size);
				tfu_flush(big_tfu.cpu, l.size);
				const uint64_t c0 = k_cycle_get_64();
				rc = tfu_run(&job, &st);
				const uint64_t d = k_cycle_get_64() - c0;
				jobs++;
				tb = MIN(tb, d);
				tw = MAX(tw, d);
				poll_best = MIN(poll_best, st.us);
			}
			if (rc == 0) {
				tfu_invd(big_tfu.cpu, l.size);
				tfu_case_compare(big_cpu.cpu, big_tfu.cpu, &l, &c);
			}
			printk("gl_test: tfuprobe: 1920x720 as %s: TFU best %llu worst %llu us (polled %u us), rc %d; "
			       "%u of %u texels wrong (unwritten %u), pad+%u; resets %u\n",
			       tnames[t], (unsigned long long)tfu_us(tb), (unsigned long long)tfu_us(tw), poll_best, rc,
			       c.texel_wrong, c.texels, c.texel_unwritten, c.pad_written, osal_v3d_recoveries() - rec0);
			ok &= gl_test_check(rc == 0 && c.texels == w * h && c.texel_wrong == 0, c.texel_wrong, 0);
		}
	}
out:
	tfu_buf_free(&src);
	tfu_buf_free(&dst);
	tfu_buf_free(&big_src);
	tfu_buf_free(&big_tfu);
	tfu_buf_free(&big_cpu);
	if (osal_v3d_recoveries() != rec0) {
		printk("gl_test: tfuprobe: %u GPU resets during the case: not a clean PASS\n",
		       osal_v3d_recoveries() - rec0);
		ok = false;
	}
	printk("gl_test: tfuprobe: %u TFU jobs run\n", jobs);
	if (jobs == 0) {
		printk("gl_test: tfuprobe: NOT EXERCISED: no TFU job ran\n");
		ok = false;
	}
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 115: a cube map on both filters against dEQP's reference ---------
 *
 * 317 of the texture mustpass cases sample a cube map, and nothing on the
 * host can say which face and which texels the TMU reads. The scene, the
 * face table, the reference (dEQP's selectCubeFace, projectToFace and 2D
 * sample, tools/v3d/tex_filter_ref.h) and the verdict are
 * tools/v3d/tex_cube_case.h, which tools/v3d/tex_filter_ref_check.c checks on
 * the host; this case only draws and reads back. Per face and per filter
 * (NEAREST, LINEAR), a quad of TCC_SIZE x TCC_SIZE pixels at the lower-left
 * corner whose VS gives the direction major + x * sx + y * ty. V3D only for
 * now: the header is on v3d_gl_test's include path only. */
#include "tex_cube_case.h"



static int case_texcube(void)
{
	GLuint prog, tex = 0;
	GLint lp, l_major, l_sx, l_ty;
	struct puv_vertex q[6];
	bool ok = true;

	tcc_make_faces();
	if (!tcc_controls(printk))
		return -1;
	prog = link_program(gl_test_texcube_vs_src, gl_test_texcube_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	lp = glGetAttribLocation(prog, "a_pos");
	l_major = glGetUniformLocation(prog, "u_major");
	l_sx = glGetUniformLocation(prog, "u_sx");
	l_ty = glGetUniformLocation(prog, "u_ty");
	if (lp < 0 || l_major < 0 || l_sx < 0 || l_ty < 0) {
		pr_err("texcube: a_pos %d u_major %d u_sx %d u_ty %d\n", lp, l_major, l_sx, l_ty);
		glDeleteProgram(prog);
		return -1;
	}
	glUniform1i(glGetUniformLocation(prog, "u_s"), 0);
	glActiveTexture(GL_TEXTURE0);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	/* face f to POSITIVE_X + f, row 0 of the upload is t = 0, as the
	 * header's reference assumes */
	for (int f = 0; f < 6; f++)
		glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + (GLenum)f, 0, GL_RGBA, TCC_TEX, TCC_TEX, 0,
			     GL_RGBA, GL_UNSIGNED_BYTE, tcc_texels[f]);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("texcube upload")) {
		glDeleteTextures(1, &tex);
		glDeleteProgram(prog);
		return -1;
	}

	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glEnableVertexAttribArray((GLuint)lp);
	glViewport(0, 0, TCC_SIZE, TCC_SIZE);

	for (int i = 0; i < TCC_DRAWS; i++) {
		const int face = i / 2;
		const bool linear = (i & 1) != 0;
		const struct tcc_face *fc = &tcc_faces[face];
		struct tcc_result r;

		glUniform3f(l_major, fc->major[0], fc->major[1], fc->major[2]);
		glUniform3f(l_sx, fc->sx[0], fc->sx[1], fc->sx[2]);
		glUniform3f(l_ty, fc->ty[0], fc->ty[1], fc->ty[2]);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, linear ? GL_LINEAR : GL_NEAREST);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, linear ? GL_LINEAR : GL_NEAREST);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("texcube draw")) {
			ok = false;
			break;
		}
		read_topdown();
		/* pixel (x, y) in GL window coordinates, from the lower-left
		 * corner; s_px is top-down, and its bytes are R, G, B, A */
		tcc_result_init(&r);
		for (int y = 0; y < TCC_SIZE; y++)
			for (int x = 0; x < TCC_SIZE; x++) {
				uint8_t px[4];

				memcpy(px, &s_px[(TH - 1u - (uint32_t)y) * TW + (uint32_t)x], 4);
				tcc_check_pixel(face, linear, x, y, px, &r);
			}
		ok &= gl_test_check(tcc_verdict(face, linear, &r, printk), (uint32_t)r.over, 0);
	}
	glDisableVertexAttribArray((GLuint)lp);
	glViewport(0, 0, TW, TH);
	glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 118: mipmaps against dEQP's reference ----------------------------
 *
 * A regression pin for the V3D's mip chains: dEQP's texture.mipmap.2d cases
 * pass on the Pi 4 (T3, a3f3c39f), and this case keeps that true in the
 * gl_test set. The scene, the draws, dEQP's LOD bounds and level choice, and
 * the verdict are tools/v3d/tex_mip_case.h, which
 * tools/v3d/tex_filter_ref_check.c checks on the host; this case only uploads,
 * draws and reads back. Texture U: 64x64, 7 levels of one colour each,
 * uploaded one by one. Texture G: a one-texel checker at level 0, the rest by
 * glGenerateMipmap. Each draw is a quad of size x size pixels at the
 * lower-left corner, texture coordinates 0..1, under one of the four mipmap
 * min filters. V3D only for now: the header is on v3d_gl_test's include path
 * only. */
#include "tex_mip_case.h"

static int case_texmip(void)
{
	static const GLint min_filter[4] = { GL_NEAREST_MIPMAP_NEAREST, GL_LINEAR_MIPMAP_NEAREST,
					     GL_NEAREST_MIPMAP_LINEAR, GL_LINEAR_MIPMAP_LINEAR };
	GLuint prog, tex[2] = { 0, 0 };
	struct puv_vertex q[6];
	bool ok = true;

	if (!tmc_controls(printk))
		return -1;
	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glGenTextures(2, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	/* texture U: every level uploaded, one colour each */
	glBindTexture(GL_TEXTURE_2D, tex[0]);
	for (int k = 0; k < TMC_LEVELS; k++) {
		int n = tmc_make_level(k);

		glTexImage2D(GL_TEXTURE_2D, k, GL_RGBA, n, n, 0, GL_RGBA, GL_UNSIGNED_BYTE, tmc_texels);
	}
	/* texture G: level 0 only, the chain by glGenerateMipmap */
	glBindTexture(GL_TEXTURE_2D, tex[1]);
	tmc_make_checker();
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TMC_TEX, TMC_TEX, 0, GL_RGBA, GL_UNSIGNED_BYTE, tmc_texels);
	glGenerateMipmap(GL_TEXTURE_2D);
	if (!check_gl_error("texmip upload")) {
		glDeleteTextures(2, tex);
		return -1;
	}

	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	for (int i = 0; i < TMC_DRAWS; i++) {
		const struct tmc_draw *d = &tmc_draws[i];
		struct tmc_result r;

		glBindTexture(GL_TEXTURE_2D, tex[d->generated ? 1 : 0]);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min_filter[d->min]);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glViewport(0, 0, d->size, d->size);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("texmip draw")) {
			ok = false;
			break;
		}
		read_topdown();
		/* pixel (x, y) in GL window coordinates, from the lower-left
		 * corner; s_px is top-down, and its bytes are R, G, B, A */
		tmc_result_init(&r);
		for (int y = 0; y < d->size; y++)
			for (int x = 0; x < d->size; x++) {
				uint8_t px[4];

				memcpy(px, &s_px[(TH - 1u - (uint32_t)y) * TW + (uint32_t)x], 4);
				tmc_check_pixel(d, x, y, px, &r);
			}
		ok &= gl_test_check(tmc_verdict(d, &r, printk), r.invalid, 0);
	}
	glViewport(0, 0, TW, TH);
	glDeleteTextures(2, tex);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 133: cube-map mipmaps against dEQP's reference -----------------
 *
 * dEQP-GLES2 texture.mipmap.cube.{basic,bias,projected} fail on the Pi 4
 * (batch 23), nearest_nearest included, where a NEAREST sample never reads
 * across a face and seamless filtering cannot matter. This case asks what
 * is left, on face interiors only: does each face's level L sit where the
 * TMU reads it, and does the TMU pick the level dEQP allows? The scene (a
 * 64x64 cube, 7 levels, one colour per face and level, dEQP's formula), the
 * draws (per face, NEAREST_MIPMAP_NEAREST at LODs that allow exactly level
 * 0..4, LINEAR_MIPMAP_LINEAR blending 1-2 and 2-3), dEQP's cube LOD bounds
 * and the verdict are tools/v3d/tex_cubemip_case.h, which
 * tools/v3d/tex_filter_ref_check.c checks on the host; this case only
 * uploads, draws and reads back. Each draw's line also names the (face,
 * level) its centre pixel reads. V3D only for now: the header is on
 * v3d_gl_test's include path only. */
#include "tex_cubemip_case.h"


static int case_texcubemip(void)
{
	GLuint prog, tex = 0;
	GLint lp, l_major, l_sx, l_ty, l_h;
	struct puv_vertex q[6];
	bool ok = true;

	if (!tcm_controls(printk))
		return -1;
	prog = link_program(gl_test_texcubemip_vs_src, gl_test_texcube_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	lp = glGetAttribLocation(prog, "a_pos");
	l_major = glGetUniformLocation(prog, "u_major");
	l_sx = glGetUniformLocation(prog, "u_sx");
	l_ty = glGetUniformLocation(prog, "u_ty");
	l_h = glGetUniformLocation(prog, "u_h");
	if (lp < 0 || l_major < 0 || l_sx < 0 || l_ty < 0 || l_h < 0) {
		pr_err("texcubemip: a_pos %d u_major %d u_sx %d u_ty %d u_h %d\n", lp, l_major, l_sx, l_ty, l_h);
		glDeleteProgram(prog);
		return -1;
	}
	glUniform1i(glGetUniformLocation(prog, "u_s"), 0);
	glActiveTexture(GL_TEXTURE0);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	/* face by face, level by level, as dEQP's TextureCube::upload */
	for (int f = 0; f < 6; f++)
		for (int l = 0; l < TCM_LEVELS; l++) {
			int n = tcm_make(f, l);

			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + (GLenum)f, l, GL_RGBA, n, n, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, tcm_texels);
		}
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("texcubemip upload")) {
		glDeleteTextures(1, &tex);
		glDeleteProgram(prog);
		return -1;
	}

	puv_quad(q);
	upload_vbo(q, sizeof(q));
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glEnableVertexAttribArray((GLuint)lp);

	for (int i = 0; i < TCM_DRAWS; i++) {
		const int face = i / TCM_PER_FACE;
		const struct tcm_draw *d = &tcm_face_draws[i % TCM_PER_FACE];
		const struct tcc_face *fc = &tcc_faces[face];
		struct tcm_result r;

		glUniform3f(l_major, fc->major[0], fc->major[1], fc->major[2]);
		glUniform3f(l_sx, fc->sx[0], fc->sx[1], fc->sx[2]);
		glUniform3f(l_ty, fc->ty[0], fc->ty[1], fc->ty[2]);
		glUniform1f(l_h, d->h);
		glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER,
				d->linear ? GL_LINEAR_MIPMAP_LINEAR : GL_NEAREST_MIPMAP_NEAREST);
		glViewport(0, 0, d->size, d->size);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("texcubemip draw")) {
			ok = false;
			break;
		}
		read_topdown();
		/* pixel (x, y) in GL window coordinates, from the lower-left
		 * corner; s_px is top-down, and its bytes are R, G, B, A */
		tcm_result_init(&r);
		for (int y = 0; y < d->size; y++)
			for (int x = 0; x < d->size; x++) {
				uint8_t px[4];

				memcpy(px, &s_px[(TH - 1u - (uint32_t)y) * TW + (uint32_t)x], 4);
				tcm_check_pixel(face, d, x, y, px, &r);
			}
		ok &= gl_test_check(tcm_verdict(face, d, &r, printk), r.invalid, 0);
	}
	glDisableVertexAttribArray((GLuint)lp);
	glViewport(0, 0, TW, TH);
	glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 134: dEQP's thin cube-mipmap cells -------------------------------
 *
 * dEQP-GLES2 texture.mipmap.cube.basic.nearest_nearest fails on the Pi 4 in
 * two columns only, the edges of a 17x128 and a 7x128 cell, where the
 * pixels read their face one level finer than allowed; case 133's square
 * quads pass. This case draws those thin cells (at dEQP's pixel parity) and
 * their turned twins on every face, NEAREST_MIPMAP_NEAREST, twice: sampling
 * the direction as interpolated, then divided in the shader by its largest
 * component (Mesa's nir_normalize_cubemap_coords). Each draw's line splits
 * the invalid pixels into those whose 2x2 quad the cell cuts and the rest.
 * The scene, cells and verdict are tools/v3d/tex_cubethin_case.h, checked
 * on the host by tools/v3d/tex_filter_ref_check.c. */
#include "tex_cubethin_case.h"


static int case_texcubethin(void)
{
	GLuint prog[TCT_ARMS] = { 0, 0 }, tex = 0;
	struct puv_vertex q[6];
	/* a GL error stops the case; a failing verdict does not: every draw
	 * runs and prints its line, so both arms are always compared */
	bool ok = true, broken = false;

	if (!tct_controls(printk))
		return -1;
	prog[0] = link_program(gl_test_texcubemip_vs_src, gl_test_texcube_fs_src);
	prog[1] = link_program(gl_test_texcubemip_vs_src, gl_test_texcubenorm_fs_src);
	if (!prog[0] || !prog[1]) {
		pr_err("texcubethin: link failed (as-is %u, normalized %u)\n", prog[0], prog[1]);
		for (int a = 0; a < TCT_ARMS; a++)
			if (prog[a])
				glDeleteProgram(prog[a]);
		return -1;
	}
	glActiveTexture(GL_TEXTURE0);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	/* face by face, level by level, as dEQP's TextureCube::upload */
	for (int f = 0; f < 6; f++)
		for (int l = 0; l < TCM_LEVELS; l++) {
			int n = tcm_make(f, l);

			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + (GLenum)f, l, GL_RGBA, n, n, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, tcm_texels);
		}
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("texcubethin upload")) {
		broken = true;
		goto out;
	}
	puv_quad(q);
	upload_vbo(q, sizeof(q));

	for (int a = 0; a < TCT_ARMS && !broken; a++) {
		GLint lp, l_major, l_sx, l_ty, l_h;

		glUseProgram(prog[a]);
		lp = glGetAttribLocation(prog[a], "a_pos");
		l_major = glGetUniformLocation(prog[a], "u_major");
		l_sx = glGetUniformLocation(prog[a], "u_sx");
		l_ty = glGetUniformLocation(prog[a], "u_ty");
		l_h = glGetUniformLocation(prog[a], "u_h");
		if (lp < 0 || l_major < 0 || l_sx < 0 || l_ty < 0 || l_h < 0) {
			pr_err("texcubethin: arm %d a_pos %d u_major %d u_sx %d u_ty %d u_h %d\n", a, lp, l_major,
			       l_sx, l_ty, l_h);
			broken = true;
			break;
		}
		glUniform1i(glGetUniformLocation(prog[a], "u_s"), 0);
		glUniform1f(l_h, 1.0f); /* the whole face, as dEQP's cells */
		glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex),
				      (const void *)0);
		glEnableVertexAttribArray((GLuint)lp);
		for (int face = 0; face < 6 && !broken; face++)
			for (int i = 0; i < TCT_CELLS; i++) {
				const struct tct_cell *c = &tct_cells[i];
				const struct tcc_face *fc = &tcc_faces[face];
				struct tct_result r;

				glUniform3f(l_major, fc->major[0], fc->major[1], fc->major[2]);
				glUniform3f(l_sx, fc->sx[0], fc->sx[1], fc->sx[2]);
				glUniform3f(l_ty, fc->ty[0], fc->ty[1], fc->ty[2]);
				glViewport(0, 0, TW, TH);
				glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
				glClear(GL_COLOR_BUFFER_BIT);
				glViewport(c->x0, c->y0, c->w, c->h);
				glDrawArrays(GL_TRIANGLES, 0, 6);
				glFinish();
				if (!check_gl_error("texcubethin draw")) {
					broken = true;
					break;
				}
				read_topdown();
				/* pixel (x, y) in GL window coordinates, from the
				 * lower-left corner; s_px is top-down, R, G, B, A */
				tct_result_init(&r);
				for (int y = c->y0; y < c->y0 + c->h; y++)
					for (int x = c->x0; x < c->x0 + c->w; x++) {
						uint8_t px[4];

						memcpy(px, &s_px[(TH - 1u - (uint32_t)y) * TW + (uint32_t)x], 4);
						tct_check_pixel(face, c, x, y, px, &r);
					}
				ok &= gl_test_check(tct_verdict(a, face, c, &r, printk), r.bad_split + r.bad_whole, 0);
			}
		glDisableVertexAttribArray((GLuint)lp);
	}
out:
	glViewport(0, 0, TW, TH);
	glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	for (int a = 0; a < TCT_ARMS; a++)
		glDeleteProgram(prog[a]);
	return ok && !broken ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 135: L8, A8 and LA8 sampled narrow ----------------------------------
 *
 * The V3D keeps GL_LUMINANCE, GL_ALPHA and GL_LUMINANCE_ALPHA at 1, 1 and 2
 * bytes a texel and samples them as R8 (XXX1, 000X) and RG8 (XXXY), as Mesa
 * does; the tiled layout depends on the texel size. Per format: a 64x64
 * chain of 7 levels drawn level by level at one texel a pixel, a 13x7 level,
 * and a 16x16 cube of 5 levels drawn per face at levels 0 and 2. Every pixel
 * is judged against the hashed bytes of the texel it reads. The draws,
 * texels and verdict are tools/v3d/tex_narrow_case.h, checked on the host by
 * tools/v3d/tex_filter_ref_check.c (part 9). */
#include "tex_narrow_case.h"

static uint8_t tnc_img[TNC_CHAIN * TNC_CHAIN * 2];

static GLenum tnc_gl_format(enum tnc_format f)
{
	return f == TNC_L8 ? GL_LUMINANCE : f == TNC_A8 ? GL_ALPHA : GL_LUMINANCE_ALPHA;
}

/* upload level `level` of layer `layer` of texture `kind` to `target` */
static void tnc_upload(GLenum target, enum tnc_format f, enum tnc_kind kind, int layer, int level)
{
	int w = tnc_level_w(kind, level), h = tnc_level_h(kind, level);
	unsigned bpp = tnc_bpp(f);
	GLenum fmt = tnc_gl_format(f);

	for (int y = 0; y < h; y++)
		for (int x = 0; x < w; x++)
			for (unsigned k = 0; k < bpp; k++)
				tnc_img[((unsigned)(y * w + x)) * bpp + k] = tnc_byte(kind, layer, level, x, y, (int)k);
	glTexImage2D(target, level, (GLint)fmt, w, h, 0, fmt, GL_UNSIGNED_BYTE, tnc_img);
}

static void tnc_params(GLenum target, bool mip)
{
	glTexParameteri(target, GL_TEXTURE_MIN_FILTER, mip ? GL_NEAREST_MIPMAP_NEAREST : GL_NEAREST);
	glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
}

static int case_texnarrow(void)
{
	GLuint p2d, pcube, tex[3] = { 0, 0, 0 };
	GLint lp, l_major, l_sx, l_ty;
	struct puv_vertex q[6];
	bool ok = true, broken = false;
	unsigned draws = 0;

	if (!tnc_controls(printk))
		return -1;
	p2d = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	pcube = link_program(gl_test_texcube_vs_src, gl_test_texcube_fs_src);
	if (!p2d || !pcube) {
		pr_err("texnarrow: link failed (2D %u, cube %u)\n", p2d, pcube);
		glDeleteProgram(p2d);
		glDeleteProgram(pcube);
		return -1;
	}
	lp = glGetAttribLocation(pcube, "a_pos");
	l_major = glGetUniformLocation(pcube, "u_major");
	l_sx = glGetUniformLocation(pcube, "u_sx");
	l_ty = glGetUniformLocation(pcube, "u_ty");
	if (lp < 0 || l_major < 0 || l_sx < 0 || l_ty < 0) {
		pr_err("texnarrow: a_pos %d u_major %d u_sx %d u_ty %d\n", lp, l_major, l_sx, l_ty);
		glDeleteProgram(p2d);
		glDeleteProgram(pcube);
		return -1;
	}
	glUseProgram(pcube);
	glUniform1i(glGetUniformLocation(pcube, "u_s"), 0);
	glActiveTexture(GL_TEXTURE0);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);

	for (int fi = 0; fi < TNC_FORMATS && !broken; fi++) {
		const enum tnc_format f = (enum tnc_format)fi;

		glGenTextures(3, tex);
		glBindTexture(GL_TEXTURE_2D, tex[0]);
		for (int level = 0; level < TNC_CHAIN_LEVELS; level++)
			tnc_upload(GL_TEXTURE_2D, f, TNC_KIND_CHAIN, 0, level);
		tnc_params(GL_TEXTURE_2D, true);
		glBindTexture(GL_TEXTURE_2D, tex[1]);
		tnc_upload(GL_TEXTURE_2D, f, TNC_KIND_ODD, 0, 0);
		tnc_params(GL_TEXTURE_2D, false);
		glBindTexture(GL_TEXTURE_CUBE_MAP, tex[2]);
		for (int level = 0; level < TNC_CUBE_LEVELS; level++)
			for (int face = 0; face < 6; face++)
				tnc_upload(GL_TEXTURE_CUBE_MAP_POSITIVE_X + (GLenum)face, f, TNC_KIND_CUBE, face, level);
		tnc_params(GL_TEXTURE_CUBE_MAP, true);
		if (!check_gl_error("texnarrow upload")) {
			broken = true;
			break;
		}

		for (int i = 0; i < TNC_DRAWS; i++) {
			const struct tnc_draw d = tnc_draw_of(i);
			struct tnc_result r;

			glViewport(0, 0, TW, TH);
			glClearColor(0.5f, 0.25f, 0.75f, 0.5f);
			glClear(GL_COLOR_BUFFER_BIT);
			glViewport(0, 0, d.size_w, d.size_h);
			if (d.kind == TNC_KIND_CUBE) {
				const struct tcc_face *fc = &tcc_faces[d.face];

				glUseProgram(pcube);
				glBindTexture(GL_TEXTURE_CUBE_MAP, tex[2]);
				glUniform3f(l_major, fc->major[0], fc->major[1], fc->major[2]);
				glUniform3f(l_sx, fc->sx[0], fc->sx[1], fc->sx[2]);
				glUniform3f(l_ty, fc->ty[0], fc->ty[1], fc->ty[2]);
				puv_quad(q);
				GLuint vbo = upload_vbo(q, sizeof(q));
				glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex),
						      (const void *)0);
				glEnableVertexAttribArray((GLuint)lp);
				glDrawArrays(GL_TRIANGLES, 0, 6);
				glDisableVertexAttribArray((GLuint)lp);
				glDeleteBuffers(1, &vbo);
			} else {
				glUseProgram(p2d);
				glBindTexture(GL_TEXTURE_2D, tex[d.kind == TNC_KIND_CHAIN ? 0 : 1]);
				wrap_quad(1.0f, 1.0f);
				glDrawArrays(GL_TRIANGLES, 0, 6);
				glDisableVertexAttribArray(0);
				glDisableVertexAttribArray(1);
			}
			glFinish();
			if (!check_gl_error("texnarrow draw")) {
				broken = true;
				break;
			}
			read_topdown();
			/* pixel (x, y) in GL window coordinates from the lower-left
			 * corner; s_px is top-down, its bytes R, G, B, A */
			tnc_result_init(&r);
			for (int y = 0; y < d.size_h; y++)
				for (int x = 0; x < d.size_w; x++) {
					uint8_t px[4];

					memcpy(px, &s_px[(TH - 1u - (uint32_t)y) * TW + (uint32_t)x], 4);
					tnc_check_pixel(f, &d, x, y, px, &r);
				}
			ok &= gl_test_check(tnc_verdict(f, i, &r, printk), r.bad, 0);
			draws++;
		}
		glBindTexture(GL_TEXTURE_2D, 0);
		glBindTexture(GL_TEXTURE_CUBE_MAP, 0);
		glDeleteTextures(3, tex);
	}
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glUseProgram(0);
	glDeleteProgram(p2d);
	glDeleteProgram(pcube);
	/* an empty run is not a pass: every draw of every format was judged */
	ok &= expect_int("texnarrow draws judged", (long)draws, TNC_FORMATS * TNC_DRAWS);
	return ok && !broken ? 0 : -1;
}
#endif

#if defined(GL_TEST_V3D) || !defined(GL_TEST_VIVANTE)
/* ---- case 123: depth and stencil kept apart (dEQP fbo.render FB2) ----------
 *
 * FB2 on the Pi 4 (651755c7): shared_depthbuffer.*_stencil_index8 (7) and
 * recreate_stencilbuffer.no_rebind_*_depth_component16_stencil_index8 (7)
 * drew the wrong picture. The GL core names an FBO's depth/stencil storage
 * (depth rbo << 16) | stencil rbo, and the backend kept one D24S8 buffer per
 * name: two FBOs sharing a depth renderbuffer with stencils of their own, or
 * an FBO whose stencil was replaced, got a fresh depth. v3d_split_zs(true)
 * keeps each half under its own renderbuffer (src/v3d/backend.rs zs_keys).
 * This case replays both dEQP sequences (es2fFboRenderTest.cpp:1540-1631,
 * :1985-2102) at 64x64 with the switch off and on, and reads every pixel:
 * off must draw the wrong picture (the depth lost: the last quad everywhere),
 * on the right one. Pixels within one of the diagonal where the last quad's
 * depth equals the stored 0.5 are not judged (D16 against D24 rounding).
 *
 * Each scene starts with no kept depth/stencil buffer (zsa_fresh). The
 * first board run (99ae2d63) ran the scenes back to back: the recreate
 * scene got the shared scene's renderbuffer names again, and after the
 * orphan rule its FBO's id was the shared scene's fbo B id, whose kept
 * buffer held B's sloped depth. That stale depth drew the right picture
 * by coincidence (a new renderbuffer's contents are undefined, so loading
 * it is legal; the host rig replays that sequence). Verdicts, per half:
 * shared, one buffer: the FB2 picture; shared, apart: right; recreate,
 * apart: right; recreate, one buffer: printed, not judged (what it draws
 * depends on what the backend kept under the new id).
 *
 * V3D and PowerVR, each with its backend's switch: the AM62P failed the same
 * 14 (a60104c2), and pvr_split_zs keeps the stencil in a buffer of its own
 * through CR_ISP_STENCIL_LOAD_BASE's separate-stencil enable
 * (src/pvr/backend.rs zs_keys), a pairing Mesa never uses, so on the
 * PowerVR this case is its first measurement. Not the GC7000. On the
 * PowerVR the switch does not give kept buffers back, so zsa_fresh only
 * sets it: its recreate verdicts there carry the stale-buffer caveat above. */
#ifdef GL_TEST_V3D
extern bool v3d_split_zs(bool on);
#define zsa_split v3d_split_zs
#else
extern bool pvr_split_zs(bool on);
#define zsa_split pvr_split_zs
#endif

/* The switch set to `split`, and on the V3D no kept depth/stencil buffer:
 * v3d_split_zs gives every kept buffer back when it changes, so switch away
 * and back. pvr_split_zs keeps them (see the caveat above). */
static void zsa_fresh(bool split)
{
	zsa_split(!split);
	zsa_split(split);
}



#define ZSA_RED   0xff0000ffu
#define ZSA_GREEN 0xff00ff00u
#define ZSA_BLUE  0xffff0000u

/* sglr::drawQuad from (-1, -1, z00) to (1, 1, z11): the other two corners at
 * the mean, so z is one plane over both triangles */
static void zsa_quad(GLuint prog, uint32_t rgba, float z00, float z11)
{
	const float m = 0.5f * (z00 + z11);
	const float v[18] = { -1, -1, z00, 1, -1, m, -1, 1, m, -1, 1, m, 1, -1, m, 1, 1, z11 };
	GLint lp = glGetAttribLocation(prog, "a_pos");

	glUseProgram(prog);
	glUniform4f(glGetUniformLocation(prog, "u_color"), (float)(rgba & 0xff) / 255.0f,
		    (float)((rgba >> 8) & 0xff) / 255.0f, (float)((rgba >> 16) & 0xff) / 255.0f,
		    (float)(rgba >> 24) / 255.0f);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 3, GL_FLOAT, GL_FALSE, 12, v);
	glEnableVertexAttribArray((GLuint)lp);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray((GLuint)lp);
}

static GLuint zsa_rb(GLenum format)
{
	GLuint rb;

	glGenRenderbuffers(1, &rb);
	glBindRenderbuffer(GL_RENDERBUFFER, rb);
	glRenderbufferStorage(GL_RENDERBUFFER, format, TW, TH);
	return rb;
}

static GLuint zsa_colour(void)
{
	GLuint t;

	glGenTextures(1, &t);
	glBindTexture(GL_TEXTURE_2D, t);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glBindTexture(GL_TEXTURE_2D, 0);
	return t;
}

/* The bound FBO against the picture: `pass_below` the last quad passed the
 * depth test where x + y + 1 < 64 (GL window coordinates), else where it is
 * > 64; there it is `top`, elsewhere `under`. Counts the judged pixels that
 * match the right picture and the wrong one (`top` everywhere). */
static void zsa_judge(const char *what, bool pass_below, uint32_t top, uint32_t under, uint32_t *right,
		      uint32_t *wrong, uint32_t *judged)
{
	uint32_t first = TW * TH;

	*right = *wrong = *judged = 0;
	read_topdown();
	for (uint32_t ty = 0; ty < TH; ty++)
		for (uint32_t x = 0; x < TW; x++) {
			int s = (int)(x + (TH - 1 - ty) + 1) - (int)TW;
			uint32_t got = s_px[ty * TW + x];

			if (s >= -1 && s <= 1)
				continue;
			bool drawn = pass_below ? s < 0 : s > 0;
			(*judged)++;
			if (chan_diff(got, drawn ? top : under) <= 1)
				(*right)++;
			else if (first == TW * TH)
				first = ty * TW + x;
			if (chan_diff(got, top) <= 1)
				(*wrong)++;
		}
	pr_info("gl_test: zsapart %s: %u of %u judged pixels right, %u as if the depth were lost\n", what,
	       *right, *judged, *wrong);
	if (first != TW * TH)
		pr_info("gl_test: zsapart %s: first differing (%u, %u) from the top: 0x%08x\n", what, first % TW,
		       first / TW, s_px[first]);
}

/* SharedDepthbufferTest: fbo B shares fbo A's depth, each has its stencil */
static bool zsa_shared(GLuint prog, uint32_t *right, uint32_t *wrong, uint32_t *judged, const char *what)
{
	GLuint depth = zsa_rb(GL_DEPTH_COMPONENT16), st[2], col[2], fbo[2];

	glGenFramebuffers(2, fbo);
	for (int i = 0; i < 2; i++) {
		col[i] = zsa_colour();
		st[i] = zsa_rb(GL_STENCIL_INDEX8);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col[i], 0);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, st[i]);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: zsapart: fbo %d incomplete\n", i);
			return false;
		}
	}
	glViewport(0, 0, TW, TH);
	/* B: red, depth 1, stencil 1 */
	glBindFramebuffer(GL_FRAMEBUFFER, fbo[1]);
	glClearColor(1, 0, 0, 1);
	glClearStencil(1);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glEnable(GL_DEPTH_TEST);
	/* A: cleared, a green quad at z 0 (depth 0.5) */
	glBindFramebuffer(GL_FRAMEBUFFER, fbo[0]);
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	zsa_quad(prog, ZSA_GREEN, 0.0f, 0.0f);
	/* B: a blue quad from z -1 to +1, against A's depth */
	glBindFramebuffer(GL_FRAMEBUFFER, fbo[1]);
	zsa_quad(prog, ZSA_BLUE, -1.0f, 1.0f);
	glDisable(GL_DEPTH_TEST);
	bool ok = check_gl_error("zsapart shared");
	zsa_judge(what, true, ZSA_BLUE, ZSA_RED, right, wrong, judged);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(2, fbo);
	glDeleteRenderbuffers(2, st);
	glDeleteRenderbuffers(1, &depth);
	glDeleteTextures(2, col);
	return ok;
}

/* RecreateBuffersTest<STENCIL>, no_rebind: the stencil deleted and made
 * again under its name while the fbo is not bound */
static bool zsa_recreate(GLuint prog, uint32_t *right, uint32_t *wrong, uint32_t *judged, const char *what)
{
	GLuint col = zsa_colour(), depth = zsa_rb(GL_DEPTH_COMPONENT16), st = zsa_rb(GL_STENCIL_INDEX8), fbo;

	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col, 0);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, st);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: zsapart: the recreate fbo is incomplete\n");
		return false;
	}
	glViewport(0, 0, TW, TH);
	glClearColor(1, 0, 0, 1);
	glClearStencil(0);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glEnable(GL_DEPTH_TEST);
	zsa_quad(prog, ZSA_GREEN, 0.0f, 0.0f);
	glEnable(GL_SCISSOR_TEST);
	glScissor(TW / 4, TH / 4, TW / 2, TH / 2);
	glClearStencil(1);
	glClear(GL_STENCIL_BUFFER_BIT);
	glDisable(GL_SCISSOR_TEST);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteRenderbuffers(1, &st);
	glBindRenderbuffer(GL_RENDERBUFFER, st);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, TW, TH);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glClearStencil(0);
	glClear(GL_STENCIL_BUFFER_BIT);
	glEnable(GL_STENCIL_TEST);
	glStencilFunc(GL_EQUAL, 0, 0xff);
	/* from z +1 at (-1, -1) to -1 at (1, 1): passes above the diagonal */
	zsa_quad(prog, ZSA_BLUE, 1.0f, -1.0f);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_DEPTH_TEST);
	bool ok = check_gl_error("zsapart recreate");
	zsa_judge(what, false, ZSA_BLUE, ZSA_GREEN, right, wrong, judged);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteRenderbuffers(1, &st);
	glDeleteRenderbuffers(1, &depth);
	glDeleteTextures(1, &col);
	return ok;
}

static int case_zsapart(void)
{
	GLuint prog = link_program(gl_test_zsa_vs_src, gl_test_zsa_fs_src);
	bool was, ok = true;

	if (!prog)
		return -1;
	was = zsa_split(false);
	for (int split = 0; split < 2; split++) {
		for (int scene = 0; scene < 2; scene++) {
			uint32_t right, wrong, judged;
			char what[64];

			zsa_fresh(split != 0);
			snprintf(what, sizeof(what), "%s, %s", scene ? "recreate_stencilbuffer" : "shared_depthbuffer",
				 split ? "apart" : "one buffer");
			if (!(scene ? zsa_recreate : zsa_shared)(prog, &right, &wrong, &judged, what)) {
				ok = false;
				continue;
			}
			if (split) {
				ok &= gl_test_check(judged > 0 && right == judged, right, judged);
			} else if (scene == 0) {
				/* the FB2 symptom: the depth lost, the last quad
				 * everywhere; a default that no longer draws it is
				 * news, and this case says so */
				if (wrong != judged)
					pr_info("gl_test: zsapart %s: the FB2 symptom is GONE with the halves in one "
					       "buffer\n", what);
				ok &= gl_test_check(judged > 0 && wrong == judged, wrong, judged);
			} else {
				pr_info("gl_test: zsapart %s: not judged (%s)\n", what,
					wrong == judged ? "the depth lost, as the host rig predicts from a fresh start"
					: right == judged ? "the right picture"
							  : "neither picture");
			}
		}
	}
	zsa_split(was);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 124: the binner's overflow blocks (arena step c) -------------------
 *
 * A job whose tile lists outgrow its tile pool (663,552 bytes at the default
 * budget) makes the binner ask for more memory; the OSAL hands it
 * CONFIG_YAKOGL_V3D_BIN_OVERFLOW_BLOCKS blocks of 256 KiB (v3d_wait_bin).
 * Nothing we run grows that far (Slint's 493 draws used 282,624 bytes), so
 * this case does: a 1024x1024 FBO (256 tiles of 64x64), each draw a
 * checkerboard half of 64x64 cells of 16 pixels (16 triangles in every
 * tile), in frames of N draws, N growing by half each frame so a frame
 * lands between the pool and the pool plus the blocks. Each frame is one
 * job (checked), and every pixel is read back: the cells of each parity
 * hold the colour of that parity's last draw. PASS when a frame used an
 * overflow block (v3d_bin_last) and drew right. With no blocks
 * (CONFIG_..._BIN_OVERFLOW_BLOCKS=0, the control image), the frame that runs
 * out must fail by name, counted once (osal_v3d_bin_oom_jobs); the binner is
 * then left waiting for memory, so the control is the image's last case.
 * V3D only. */
/* the jobs submitted since init always; the rest when the OSAL has a
 * reading (false otherwise) */
extern bool v3d_bin_last(uint32_t *jobs, uint32_t *overflow_used, uint32_t *overflow_blocks, uint32_t *kicks);
extern uint32_t osal_v3d_bin_oom_jobs(void);

#define BOV_SIZE  1024u
#define BOV_CELLS 64u /* cells a side, 16 pixels each */
#define BOV_TRIS  (BOV_CELLS * BOV_CELLS / 2u * 2u)
static float bov_vtx[2][BOV_TRIS * 3u * 2u];
static uint32_t bov_row[BOV_SIZE * 16u];



/* draw i's colour: distinct for every i, opaque, no channel 0 */
static uint32_t bov_colour(uint32_t i)
{
	return 0xff000000u | ((1u + (i * 37u) % 250u) << 16) | ((1u + (i * 91u) % 250u) << 8) |
	       (1u + (i * 13u) % 250u);
}

static void bov_make(void)
{
	for (uint32_t p = 0; p < 2; p++) {
		float *v = bov_vtx[p];

		for (uint32_t cy = 0; cy < BOV_CELLS; cy++)
			for (uint32_t cx = 0; cx < BOV_CELLS; cx++) {
				if (((cx + cy) & 1u) != p)
					continue;
				float x0 = (float)cx * 2.0f / BOV_CELLS - 1.0f, x1 = x0 + 2.0f / BOV_CELLS;
				float y0 = (float)cy * 2.0f / BOV_CELLS - 1.0f, y1 = y0 + 2.0f / BOV_CELLS;
				const float q[12] = { x0, y0, x1, y0, x0, y1, x0, y1, x1, y0, x1, y1 };

				memcpy(v, q, sizeof(q));
				v += 12;
			}
	}
}

/* One frame of `n` draws, alternating parity; true when every pixel holds
 * its parity's last colour. */
static bool bov_frame(GLuint prog, uint32_t n, GLint lp, GLint lc, const GLuint *vbo)
{
	uint32_t last[2] = { 0xff000000u, 0xff000000u }, bad = 0, first_x = 0, first_y = 0, got1 = 0;

	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	glUseProgram(prog);
	for (uint32_t i = 0; i < n; i++) {
		uint32_t p = i & 1u, c = bov_colour(i);

		glUniform4f(lc, (float)(c & 0xff) / 255.0f, (float)((c >> 8) & 0xff) / 255.0f,
			    (float)((c >> 16) & 0xff) / 255.0f, 1.0f);
		glBindBuffer(GL_ARRAY_BUFFER, vbo[p]);
		glVertexAttribPointer((GLuint)lp, 2, GL_FLOAT, GL_FALSE, 8, (const void *)0);
		glDrawArrays(GL_TRIANGLES, 0, (GLsizei)(BOV_TRIS * 3u));
		last[p] = c;
	}
	glFinish();
	/* read back in strips of 16 rows */
	for (uint32_t y0 = 0; y0 < BOV_SIZE; y0 += 16u) {
		glReadPixels(0, (GLint)y0, BOV_SIZE, 16, GL_RGBA, GL_UNSIGNED_BYTE, bov_row);
		for (uint32_t y = 0; y < 16u; y++)
			for (uint32_t x = 0; x < BOV_SIZE; x++) {
				uint32_t cell = (x / 16u + (y0 + y) / 16u) & 1u, got = bov_row[y * BOV_SIZE + x];

				if (chan_diff(got, last[cell]) > 1) {
					if (!bad) {
						first_x = x;
						first_y = y0 + y;
						got1 = got;
					}
					bad++;
				}
			}
	}
	if (bad)
		printk("gl_test: binovf %u draws: %u of %u pixels wrong; first (%u, %u) from the bottom = 0x%08x, "
		       "want 0x%08x\n", n, bad, BOV_SIZE * BOV_SIZE, first_x, first_y, got1,
		       last[((first_x / 16u) + (first_y / 16u)) & 1u]);
	return bad == 0;
}

static int case_binovf(void)
{
	GLuint prog = link_program(gl_test_bov_vs_src, gl_test_bov_fs_src), tex, fbo, vbo[2];
	GLint lp, lc;
	bool ok = false, done = false;
	uint32_t n = 8, oom0 = osal_v3d_bin_oom_jobs();

	if (!prog)
		return -1;
	bov_make();
	lp = glGetAttribLocation(prog, "a_pos");
	lc = glGetUniformLocation(prog, "u_color");
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, BOV_SIZE, BOV_SIZE, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glViewport(0, 0, BOV_SIZE, BOV_SIZE);
	/* the vertices in buffers, uploaded once: client arrays would be
	 * copied into the job's data arena at every draw and split the frame */
	glGenBuffers(2, vbo);
	for (int p = 0; p < 2; p++) {
		glBindBuffer(GL_ARRAY_BUFFER, vbo[p]);
		glBufferData(GL_ARRAY_BUFFER, sizeof(bov_vtx[p]), bov_vtx[p], GL_STATIC_DRAW);
	}
	glEnableVertexAttribArray((GLuint)lp);
	while (!done && n <= 1000u) {
		uint32_t used = 0, blocks = 0, kicks0 = 0, kicks = 0;

		(void)v3d_bin_last(NULL, NULL, NULL, &kicks0);
		bool right = bov_frame(prog, n, lp, lc, vbo);
		uint32_t oom = osal_v3d_bin_oom_jobs() - oom0;

		if (oom) {
			/* the pool ran dry: named by the OSAL, counted once */
			v3d_bin_last(NULL, NULL, &blocks, NULL);
			printk("gl_test: binovf %u draws: the binner ran out of memory (%u overflow blocks); "
			       "counted %u\n", n, blocks, oom);
			ok = gl_test_check(blocks == 0 && oom == 1, oom, 1);
			if (blocks)
				printk("gl_test: binovf: ran DRY with %u blocks: the frame grew past the pool and all "
				       "its blocks in one step\n", blocks);
			done = true;
			break;
		}
		if (!v3d_bin_last(NULL, &used, &blocks, &kicks)) {
			printk("gl_test: binovf: the OSAL gives no binner pool reading\n");
			ok = gl_test_check(false, 0, 1);
			done = true;
			break;
		}
		printk("gl_test: binovf %u draws: %u job(s), overflow blocks %u of %u, picture %s\n", n,
		       kicks - kicks0, used, blocks, right ? "right" : "WRONG");
		if (kicks - kicks0 != 1) {
			printk("gl_test: binovf: the frame split into %u jobs (an arena was full): its pool never "
			       "grows past one job's\n", kicks - kicks0);
			ok = gl_test_check(false, kicks - kicks0, 1);
			done = true;
			break;
		}
		if (!right) {
			ok = gl_test_check(false, used, 0);
			done = true;
			break;
		}
		if (used >= 1) {
			ok = gl_test_check(true, used, 1);
			done = true;
			break;
		}
		n += n / 2u;
	}
	if (!done) {
		printk("gl_test: binovf: NOT EXERCISED: no frame up to 1000 draws used an overflow block\n");
		ok = gl_test_check(false, 0, 1);
	}
	glDisableVertexAttribArray((GLuint)lp);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glDeleteBuffers(2, vbo);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 125: glTexSubImage2D into a texture rendered to (dEQP FB2) --------
 *
 * FB2 on the Pi 4 (651755c7) failed all four fbo.render.texsubimage cases
 * (the i.MX8MP fails them too); the host rig finds the order right. The
 * first replica (99ae2d63: RGBA source, NEAREST, a solid sub-image, drawn
 * into another FBO) passed on the board, so this one follows
 * es2fFboRenderTest.cpp:1659-1795 line by line where it differed: the
 * source uploaded as GL_RGB, LINEAR min (mag LINEAR by default) with REPEAT,
 * the texture of the case's format (rgb and rgba), the sub-image in that
 * format with a gradient (a solid one cannot show a flip or a shift), the
 * fbo bound during the sub-image, after_render and between_render (the
 * blended second render), then the texture drawn LINEAR on the window, as
 * dEQP does, and read back. Both the texture (read from its fbo) and the
 * window are judged against a reference that does GL's LINEAR/REPEAT in C,
 * with the mean difference per channel and the box of differing pixels.
 * V3D only. */
#define TSF 128u
static uint32_t tsf_px[TSF * TSF];
static uint32_t tsf_want_tex[TSF * TSF];



/* dEQP's fourQuads stand-in: the 64x64 source, one colour per quadrant,
 * GL coordinates (row 0 at the bottom) */
static uint32_t tsf_quad(uint32_t x, uint32_t y)
{
	static const uint32_t q[4] = { 0xff0000ffu, 0xff00ff00u, 0xffff0000u, 0xffffffffu };

	return q[(y >= 32 ? 2 : 0) + (x >= 32 ? 1 : 0)];
}

/* the sub-image: a gradient, so a sub-image flipped, transposed or shifted
 * shows (a solid one could not); alpha varies for the RGBA variant */
static uint32_t tsf_sub(uint32_t i, uint32_t j, bool rgba)
{
	uint32_t a = rgba ? (0x80u | ((i * 2u + j) & 0x7fu)) : 0xffu;

	return (a << 24) | (0x80u << 16) | ((j * 4u) << 8) | (i * 4u);
}

#define TSF_BALL 0x80ff0000u /* dEQP's metaballs2 stand-in: blue, alpha 0x80 */

static float tsf_ch(uint32_t v, int c)
{
	return (float)((v >> (8 * c)) & 0xffu);
}

/* GL's LINEAR with REPEAT (ES 2.0 3.7.7) of a w x h RGBA8 image `img`, at
 * the centre of pixel (x, y) of a tw x th target, channel c */
static float tsf_lin(const uint32_t *img, uint32_t w, uint32_t h, uint32_t x, uint32_t y, uint32_t tw,
		     uint32_t th, int c)
{
	float u = ((float)x + 0.5f) * (float)w / (float)tw - 0.5f;
	float v = ((float)y + 0.5f) * (float)h / (float)th - 0.5f;
	int i0 = (int)(u + 1024.0f) - 1024, j0 = (int)(v + 1024.0f) - 1024;
	float a = u - (float)i0, b = v - (float)j0;
	uint32_t i0w = (uint32_t)(i0 + (int)w) % w, i1w = (uint32_t)(i0 + 1 + (int)w) % w;
	uint32_t j0w = (uint32_t)(j0 + (int)h) % h, j1w = (uint32_t)(j0 + 1 + (int)h) % h;

	return (1 - a) * (1 - b) * tsf_ch(img[j0w * w + i0w], c) + a * (1 - b) * tsf_ch(img[j0w * w + i1w], c) +
	       (1 - a) * b * tsf_ch(img[j1w * w + i0w], c) + a * b * tsf_ch(img[j1w * w + i1w], c);
}

static uint32_t tsf_pack(const float *f)
{
	uint32_t out = 0;

	for (int c = 0; c < 4; c++) {
		float v = f[c] + 0.5f;
		uint32_t b = v <= 0 ? 0 : v >= 255 ? 255 : (uint32_t)v;

		out |= b << (8 * c);
	}
	return out;
}

static uint32_t tsf_src[64 * 64];

/* the texture as dEQP's reference leaves it: the source magnified LINEAR,
 * the sub-image written over (32..95), and with `between` the blended
 * second render; an RGB texture reads alpha 1 */
static void tsf_expect(bool rgba, bool between)
{
	float a = tsf_ch(TSF_BALL, 3) / 255.0f;

	for (uint32_t y = 0; y < TSF; y++)
		for (uint32_t x = 0; x < TSF; x++) {
			float f[4];

			if (x >= 32 && x < 96 && y >= 32 && y < 96) {
				uint32_t s = tsf_sub(x - 32, y - 32, rgba);

				for (int c = 0; c < 4; c++)
					f[c] = tsf_ch(s, c);
			} else {
				for (int c = 0; c < 4; c++)
					f[c] = tsf_lin(tsf_src, 64, 64, x, y, TSF, TSF, c);
			}
			if (between)
				for (int c = 0; c < 3; c++)
					f[c] = tsf_ch(TSF_BALL, c) * a + f[c] * (1.0f - a);
			if (!rgba)
				f[3] = 255.0f;
			tsf_want_tex[y * TSF + x] = tsf_pack(f);
		}
}

static void tsf_draw(GLuint prog, GLuint tex)
{
	static const float v[12] = { -1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1 };
	GLint lp = glGetAttribLocation(prog, "a_pos");

	glUseProgram(prog);
	glBindTexture(GL_TEXTURE_2D, tex);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 2, GL_FLOAT, GL_FALSE, 8, v);
	glEnableVertexAttribArray((GLuint)lp);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray((GLuint)lp);
}

/* `got` (w x h, GL row order) against `want`: the mean difference per
 * channel, the box of pixels more than `tol` off in the first `chans`
 * channels, and the first of them */
static bool tsf_judge(const char *what, const uint32_t *got, const uint32_t *want, uint32_t w, uint32_t h,
		      int chans, int tol)
{
	uint64_t sum[4] = { 0 };
	uint32_t bad = 0, x0 = w, y0 = h, x1 = 0, y1 = 0, first = w * h, n = w * h;

	for (uint32_t y = 0; y < h; y++)
		for (uint32_t x = 0; x < w; x++) {
			uint32_t g = got[y * w + x], e = want[y * w + x];
			int worst = 0;

			for (int c = 0; c < 4; c++) {
				int d = (int)((g >> (8 * c)) & 0xff) - (int)((e >> (8 * c)) & 0xff);

				d = d < 0 ? -d : d;
				sum[c] += (uint64_t)d;
				if (c < chans && d > worst)
					worst = d;
			}
			if (worst > tol) {
				bad++;
				if (first == n)
					first = y * w + x;
				x0 = x < x0 ? x : x0;
				y0 = y < y0 ? y : y0;
				x1 = x > x1 ? x : x1;
				y1 = y > y1 ? y : y1;
			}
		}
	printk("gl_test: texsubfbo %s: %u of %u pixels differ; mean |diff| R %u.%02u G %u.%02u B %u.%02u "
	       "A %u.%02u\n", what, bad, n, (unsigned)(sum[0] / n), (unsigned)(sum[0] * 100 / n % 100),
	       (unsigned)(sum[1] / n), (unsigned)(sum[1] * 100 / n % 100), (unsigned)(sum[2] / n),
	       (unsigned)(sum[2] * 100 / n % 100), (unsigned)(sum[3] / n), (unsigned)(sum[3] * 100 / n % 100));
	if (bad)
		printk("gl_test: texsubfbo %s: box x %u..%u y %u..%u (GL, from the bottom); first (%u, %u) = "
		       "0x%08x, want 0x%08x\n", what, x0, x1, y0, y1, first % w, first / w, got[first], want[first]);
	return gl_test_check(bad == 0, bad, 0);
}

/* TexSubImageAfterRenderTest / TexSubImageBetweenRenderTest
 * (es2fFboRenderTest.cpp:1659-1703, :1731-1795), one format */
static bool tsf_run(GLuint prog, bool rgba, bool between)
{
	static uint8_t rgb_src[64 * 64 * 3], sub[64 * 64 * 4];
	static uint32_t ball[64 * 64], win_want[TW * TH];
	const GLenum fmt = rgba ? GL_RGBA : GL_RGB;
	const int bpp = rgba ? 4 : 3;
	GLuint quads_t, ball_t = 0, fbo_t, fbo;
	char what[96];
	bool ok = true;

	tsf_expect(rgba, between);
	for (uint32_t i = 0; i < 64 * 64; i++) {
		uint32_t q = tsf_src[i];

		rgb_src[i * 3] = (uint8_t)q;
		rgb_src[i * 3 + 1] = (uint8_t)(q >> 8);
		rgb_src[i * 3 + 2] = (uint8_t)(q >> 16);
		ball[i] = TSF_BALL;
	}
	for (uint32_t j = 0; j < 64; j++)
		for (uint32_t i = 0; i < 64; i++) {
			uint32_t s = tsf_sub(i, j, rgba);

			for (int c = 0; c < bpp; c++)
				sub[(j * 64 + i) * (uint32_t)bpp + (uint32_t)c] = (uint8_t)(s >> (8 * c));
		}
	/* FboRenderCase::iterate clears the window first (es2fFboRenderTest.cpp
	 * :662-663). That clear is a job of its own, still open when the render
	 * into the texture arrives, which then ends it and goes into the next
	 * job: the GL core had stamped the texture with the ended job, so the
	 * sub-image was written under the render and the render ran over it (the
	 * FB2 texsubimage failures, 5ff3076c's QPA pictures). Without this clear
	 * the replica could not show it. */
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glClearColor(0.125f, 0.25f, 0.5f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glActiveTexture(GL_TEXTURE0);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	if (between) {
		glGenTextures(1, &ball_t);
		glBindTexture(GL_TEXTURE_2D, ball_t);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, ball);
	}
	/* fourQuads: RGB, LINEAR min (mag LINEAR by default), REPEAT */
	glGenTextures(1, &quads_t);
	glBindTexture(GL_TEXTURE_2D, quads_t);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 64, 64, 0, GL_RGB, GL_UNSIGNED_BYTE, rgb_src);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glGenTextures(1, &fbo_t);
	glBindTexture(GL_TEXTURE_2D, fbo_t);
	glTexImage2D(GL_TEXTURE_2D, 0, (GLint)fmt, TSF, TSF, 0, fmt, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, fbo_t, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: texsubfbo: the fbo is incomplete\n");
		return false;
	}
	glViewport(0, 0, TSF, TSF);
	tsf_draw(prog, quads_t);
	/* the sub-image, the fbo still bound, as dEQP */
	glBindTexture(GL_TEXTURE_2D, fbo_t);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 32, 32, 64, 64, fmt, GL_UNSIGNED_BYTE, sub);
	if (between) {
		glEnable(GL_BLEND);
		glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
		tsf_draw(prog, ball_t);
		glDisable(GL_BLEND);
	}
	ok &= check_gl_error("texsubfbo renders");
	glReadPixels(0, 0, TSF, TSF, GL_RGBA, GL_UNSIGNED_BYTE, tsf_px);
	snprintf(what, sizeof(what), "%s_render %s, the texture (read from its fbo)", between ? "between" : "after",
		 rgba ? "rgba" : "rgb");
	ok &= tsf_judge(what, tsf_px, tsf_want_tex, TSF, TSF, 4, 3);
	/* to the screen: the texture LINEAR on the window (TW x TH), as dEQP;
	 * each pixel the mean of 2 x 2 texels; RGB judged (the window's alpha
	 * is its own) */
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	tsf_draw(prog, fbo_t);
	ok &= check_gl_error("texsubfbo to the screen");
	for (uint32_t y = 0; y < TH; y++)
		for (uint32_t x = 0; x < TW; x++) {
			float f[4];

			for (int c = 0; c < 4; c++)
				f[c] = tsf_lin(tsf_want_tex, TSF, TSF, x, y, TW, TH, c);
			win_want[y * TW + x] = tsf_pack(f);
		}
	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_rows);
	snprintf(what, sizeof(what), "%s_render %s, on the screen", between ? "between" : "after",
		 rgba ? "rgba" : "rgb");
	ok &= tsf_judge(what, s_rows, win_want, TW, TH, 3, 3);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &quads_t);
	glDeleteTextures(1, &fbo_t);
	if (ball_t)
		glDeleteTextures(1, &ball_t);
	return ok;
}

static int case_texsubfbo(void)
{
	GLuint prog = link_program(gl_test_tsf_vs_src, gl_test_tsf_fs_src);
	bool ok = true;

	if (!prog)
		return -1;
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_s"), 0);
	for (uint32_t y = 0; y < 64; y++)
		for (uint32_t x = 0; x < 64; x++)
			tsf_src[y * 64 + x] = tsf_quad(x, y);
	for (int v = 0; v < 4; v++)
		ok &= tsf_run(prog, (v & 1) != 0, (v & 2) != 0);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}
#endif

/* ---- shared by cases 129 and 197: the RGB chain and dEQP's grid ---------- */

/* texel (x, y) of level L of the 16x16 GL_RGB chain: (40 + 60 L, 16 x, 16 y) */
static uint32_t vtl_texel(uint32_t level, uint32_t x, uint32_t y)
{
	return 0xff000000u | ((16u * y) << 16) | ((16u * x) << 8) | (40u + 60u * level);
}

/* dEQP's grid (es2fVertexTextureTests.cpp:78, 316-373): 127 x 127 quads,
 * four vertices each sharing the quad's coordinate (x / 127, y / 127), two
 * triangles by 16-bit indices: 1,225,804 bytes of client arrays for one draw */
#define VTL_GRID 127u
static float vtl_gpos[VTL_GRID * VTL_GRID * 4 * 2], vtl_guv[VTL_GRID * VTL_GRID * 4 * 2];
static uint16_t vtl_gidx[VTL_GRID * VTL_GRID * 6];

/* fill the grid's three client arrays */
static void vtl_grid_fill(void)
{
	uint32_t n = 0;

	for (uint32_t y = 0; y < VTL_GRID; y++)
		for (uint32_t x = 0; x < VTL_GRID; x++) {
			const float fx0 = (float)x / VTL_GRID, fx1 = (float)(x + 1u) / VTL_GRID;
			const float fy0 = (float)y / VTL_GRID, fy1 = (float)(y + 1u) / VTL_GRID;
			const float q[4][2] = { { fx0, fy0 }, { fx1, fy0 }, { fx0, fy1 }, { fx1, fy1 } };
			const uint16_t first = (uint16_t)n;

			for (int k = 0; k < 4; k++, n++) {
				vtl_gpos[n * 2] = q[k][0] * 2.0f - 1.0f;
				vtl_gpos[n * 2 + 1] = q[k][1] * 2.0f - 1.0f;
				vtl_guv[n * 2] = (float)x / VTL_GRID;
				vtl_guv[n * 2 + 1] = (float)y / VTL_GRID;
			}
			uint16_t *ix = &vtl_gidx[(y * VTL_GRID + x) * 6];

			ix[0] = first;
			ix[1] = (uint16_t)(first + 1);
			ix[2] = (uint16_t)(first + 2);
			ix[3] = (uint16_t)(first + 1);
			ix[4] = (uint16_t)(first + 3);
			ix[5] = (uint16_t)(first + 2);
		}
}

#if defined(GL_TEST_V3D) || defined(GL_TEST_VIVANTE)
/* ---- case 129: a vertex-shader texture2DLod on a mipmapped RGB8 texture ------
 *
 * dEQP texture.vertex.2d fails 45 of 45 on the Pi 4 (8c8df5d1) while random
 * VS look-ups (one level, LINEAR, CLAMP_TO_EDGE) pass, and the compiled VS is
 * Mesa's txl sequence exactly (v3d-m5). What texture.vertex adds: GL_RGB
 * textures with full mip chains, a u_lod uniform, and two textures bound. So
 * this case isolates the texture state a VS-stage binding gets: the RGB8
 * record (swizzle, alpha 1), the mip chain's base and level count as the VS
 * TMU reads them, and which texture the VS's P0 names when two are bound.
 * - Unit 0: a 16x16 GL_RGB chain (5 levels, uploaded level by level), texel
 *   (x, y) of level L = (40 + 60 L, 16 x, 16 y); NEAREST_MIPMAP_NEAREST,
 *   CLAMP_TO_EDGE. The VS samples it with texture2DLod at lod 0, 1, 2.
 * - The same chain with a non-mipmap min filter (NEAREST) at lod 1 and 2:
 *   GL reads level 0 whatever the lod. v3d-tex's reading: we copy only
 *   level 0 for a non-mipmap filter and pin the unit to it through the
 *   sampler, where Mesa keeps the whole chain; texture2DLod turns the unit's
 *   automatic LOD off, so a unit that ignores the pin reads past the copy.
 * - Unit 1: a 4x4 GL_RGB chain of 3 levels, one colour each, sampled by the
 *   FS (u_pick 1) at level 0, and bound through every pass.
 * Beside the first wrong point the case prints what levels 0..2 of both
 * textures hold there: the other texture's level 0 means a wrong record;
 * level L's colour on a non-mipmap draw, the pin ignored; neither, reading
 * past the copy; the neighbouring texel of the right level, coordinates.
 * - An 8x8 grid of 1-pixel points, each sampling at its own (u, v) =
 *   ((i + 0.5) / 8, (j + 0.5) / 8): every point checked, and the pixels
 *   between them must stay black.
 * Printed: per lod, the points right, and the first wrong one with its
 * expected value. A constant wrong colour means state (level, swizzle, the
 * other texture); a wrong G/B that varies means coordinates. V3D only. */

/* The constant pass's VS (the lead's instrument (ii), 2026-09-25): the same
 * look-up with no value the draw feeds it, the coordinate texel (8, 8)'s
 * centre and the lod a literal, so a fault here does not come from the
 * attribute or the u_lod uniform. a_uv still places the points. */

/* The grid pass's VS (dEQP texture.vertex.2d's shape): the position and the
 * coordinate each from a client array, the look-up at lod 0. */


/* unit 1's levels, one colour each */
static const uint32_t vtl_b[3] = { 0xfffac80au, 0xff33550bu, 0xff99220cu };

/* the level-L texel point (i, j) samples (NEAREST) */
static uint32_t vtl_want(uint32_t level, uint32_t i, uint32_t j)
{
	uint32_t size = 16u >> level;

	return vtl_texel(level, (2u * i + 1u) * size / 16u, (2u * j + 1u) * size / 16u);
}

/* the texel of the control texture quad (qx, qy) reads (NEAREST, CLAMP) */
static uint32_t vtl_grid_texel(uint32_t qx, uint32_t qy)
{
	uint32_t tx = qx * 16u / VTL_GRID, ty = qy * 16u / VTL_GRID;

	return vtl_texel(0, tx > 15u ? 15u : tx, ty > 15u ? 15u : ty);
}

/* The grid pass: the whole draw, every pixel of the window against the
 * texel of the quad under it (one quad either way allowed, for the pixels on
 * a quad's edge). Without room for the draw's data it is GL_OUT_OF_MEMORY
 * and nothing is drawn (texture.vertex.2d, 2026-09-25). */
static bool vtl_grid_pass(GLuint prog)
{
	GLint lp = glGetAttribLocation(prog, "a_pos"), lu = glGetAttribLocation(prog, "a_uv");
	uint32_t bad = 0, fx = 0, fy = 0, fgot = 0;
	GLenum err;

	if (lp < 0 || lu < 0) {
		pr_err("vstexlod grid: a_pos %d a_uv %d\n", lp, lu);
		return false;
	}
	vtl_grid_fill();
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 2, GL_FLOAT, GL_FALSE, 0, vtl_gpos);
	glVertexAttribPointer((GLuint)lu, 2, GL_FLOAT, GL_FALSE, 0, vtl_guv);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lu);
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawElements(GL_TRIANGLES, (GLsizei)(VTL_GRID * VTL_GRID * 6), GL_UNSIGNED_SHORT, vtl_gidx);
	glFinish();
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lu);
	err = glGetError();
	if (err != GL_NO_ERROR) {
		pr_info("gl_test: vstexlod grid: GL error 0x%x after dEQP's 127x127 grid draw (%u bytes of client "
		       "arrays)%s\n", err, (unsigned)(sizeof(vtl_gpos) + sizeof(vtl_guv) + sizeof(vtl_gidx)),
		       err == GL_OUT_OF_MEMORY ? ": the draw did not fit the job's data" : "");
		return gl_test_check(false, err, GL_NO_ERROR);
	}
	read_topdown();
	for (uint32_t ty = 0; ty < TH; ty++)
		for (uint32_t x = 0; x < TW; x++) {
			uint32_t gy = TH - 1u - ty, got = s_px[ty * TW + x];
			uint32_t qx = (2u * x + 1u) * VTL_GRID / (2u * TW), qy = (2u * gy + 1u) * VTL_GRID / (2u * TH);
			bool right = false;

			for (int dy = -1; dy <= 1 && !right; dy++)
				for (int dx = -1; dx <= 1 && !right; dx++) {
					int cx = (int)qx + dx, cy = (int)qy + dy;

					if (cx < 0 || cy < 0 || cx >= (int)VTL_GRID || cy >= (int)VTL_GRID)
						continue;
					right = chan_diff(got, vtl_grid_texel((uint32_t)cx, (uint32_t)cy)) <= 1;
				}
			if (!right && bad++ == 0) {
				fx = x;
				fy = gy;
				fgot = got;
			}
		}
	pr_info("gl_test: vstexlod grid (dEQP's 127x127, %u bytes of client arrays, one draw): %u of %u pixels "
	       "wrong\n", (unsigned)(sizeof(vtl_gpos) + sizeof(vtl_guv) + sizeof(vtl_gidx)), bad, TW * TH);
	if (bad)
		pr_info("gl_test: vstexlod grid: first wrong (%u, %u) from the bottom = 0x%08x, want 0x%08x\n", fx, fy,
		       fgot, vtl_grid_texel((2u * fx + 1u) * VTL_GRID / (2u * TW), (2u * fy + 1u) * VTL_GRID / (2u * TH)));
	return gl_test_check(bad == 0, bad, 0);
}

static int case_vstexlod(void)
{
	static uint8_t lv[16 * 16 * 3];
	static float uv[64 * 2];
	GLuint prog = link_program(gl_test_vtl_vs_src, gl_test_vtl_fs_src), tex[3];
	GLuint progk = link_program(gl_test_vtl_const_vs_src, gl_test_vtl_fs_src);
	GLint la, l_lod, l_pick, lak;
	bool ok = true;

	if (!prog || !progk) {
		glDeleteProgram(prog);
		glDeleteProgram(progk);
		return -1;
	}
	glUseProgram(progk);
	lak = glGetAttribLocation(progk, "a_uv");
	glUniform1i(glGetUniformLocation(progk, "u_vtex"), 0);
	glUniform1i(glGetUniformLocation(progk, "u_ftex"), 1);
	glUniform1f(glGetUniformLocation(progk, "u_pick"), 0.0f);
	glUseProgram(prog);
	la = glGetAttribLocation(prog, "a_uv");
	l_lod = glGetUniformLocation(prog, "u_lod");
	l_pick = glGetUniformLocation(prog, "u_pick");
	if (la < 0 || l_lod < 0 || l_pick < 0) {
		pr_err("vstexlod: a_uv %d u_lod %d u_pick %d\n", la, l_lod, l_pick);
		glDeleteProgram(prog);
		glDeleteProgram(progk);
		return -1;
	}
	glUniform1i(glGetUniformLocation(prog, "u_vtex"), 0);
	glUniform1i(glGetUniformLocation(prog, "u_ftex"), 1);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
	glGenTextures(3, tex);
	/* the control: level 0 of the chain as a texture of one level (a
	 * random VS look-up's shape, which passes on the board) */
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, tex[2]);
	for (uint32_t y = 0; y < 16; y++)
		for (uint32_t x = 0; x < 16; x++) {
			uint32_t t = vtl_texel(0, x, y);

			lv[(y * 16 + x) * 3] = (uint8_t)t;
			lv[(y * 16 + x) * 3 + 1] = (uint8_t)(t >> 8);
			lv[(y * 16 + x) * 3 + 2] = (uint8_t)(t >> 16);
		}
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 16, 16, 0, GL_RGB, GL_UNSIGNED_BYTE, lv);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glBindTexture(GL_TEXTURE_2D, tex[0]);
	for (uint32_t level = 0; level < 5; level++) {
		uint32_t size = 16u >> level;

		for (uint32_t y = 0; y < size; y++)
			for (uint32_t x = 0; x < size; x++) {
				uint32_t t = vtl_texel(level, x, y);

				lv[(y * size + x) * 3] = (uint8_t)t;
				lv[(y * size + x) * 3 + 1] = (uint8_t)(t >> 8);
				lv[(y * size + x) * 3 + 2] = (uint8_t)(t >> 16);
			}
		glTexImage2D(GL_TEXTURE_2D, (GLint)level, GL_RGB, (GLsizei)size, (GLsizei)size, 0, GL_RGB,
			     GL_UNSIGNED_BYTE, lv);
	}
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glActiveTexture(GL_TEXTURE1);
	glBindTexture(GL_TEXTURE_2D, tex[1]);
	for (uint32_t level = 0; level < 3; level++) {
		uint32_t size = 4u >> level;

		for (uint32_t k = 0; k < size * size; k++) {
			lv[k * 3] = (uint8_t)vtl_b[level];
			lv[k * 3 + 1] = (uint8_t)(vtl_b[level] >> 8);
			lv[k * 3 + 2] = (uint8_t)(vtl_b[level] >> 16);
		}
		glTexImage2D(GL_TEXTURE_2D, (GLint)level, GL_RGB, (GLsizei)size, (GLsizei)size, 0, GL_RGB,
			     GL_UNSIGNED_BYTE, lv);
	}
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glActiveTexture(GL_TEXTURE0);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	if (!check_gl_error("vstexlod upload")) {
		glDeleteTextures(3, tex);
		glDeleteProgram(prog);
		glDeleteProgram(progk);
		return -1;
	}
	for (uint32_t j = 0; j < 8; j++)
		for (uint32_t i = 0; i < 8; i++) {
			uv[(j * 8 + i) * 2] = ((float)i + 0.5f) / 8.0f;
			uv[(j * 8 + i) * 2 + 1] = ((float)j + 0.5f) / 8.0f;
		}
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)la, 2, GL_FLOAT, GL_FALSE, 8, uv);
	glEnableVertexAttribArray((GLuint)la);
	if (lak >= 0 && lak != la) {
		glVertexAttribPointer((GLuint)lak, 2, GL_FLOAT, GL_FALSE, 8, uv);
		glEnableVertexAttribArray((GLuint)lak);
	}
	glViewport(0, 0, TW, TH);
	/* passes, lightest first, each its own job: once a job hangs every
	 * later one fails, so the first pass that fails names the ingredient
	 * (the lead's order, after the 23:31 run hung at the first mipmap pass):
	 * the constant look-up (coordinate and lod literals); the one-level
	 * control; the chain without a mip filter at lod 0, then
	 * 1 and 2; the chain mipmapped at lod 0, 1, 2; a negative lod (dEQP's
	 * texture.vertex uses log2(0.3) = -1.74) both ways; the FS's unit 1 */
	static const struct {
		float lod;
		bool control, mip, fs, konst;
	} passes[] = { { 0.0f, true, false, false, true }, { 0.0f, true, false, false, false },  { 0.0f, false, false, false, false },
		       { 1.0f, false, false, false, false }, { 2.0f, false, false, false, false },
		       { 0.0f, false, true, false, false },  { 1.0f, false, true, false, false },
		       { 2.0f, false, true, false, false },  { -1.74f, false, true, false, false },
		       { -1.74f, false, false, false, false }, { 0.0f, false, true, true, false } };
	/* GL_TEST_VTL_PASSES > 0 runs only the first N passes (the V3D sample's
	 * V3D_GL_TEST_VTL_PASSES): one faulting pass's dump then fits the record */
#ifndef GL_TEST_VTL_PASSES
#define GL_TEST_VTL_PASSES 0
#endif
	size_t npass = sizeof(passes) / sizeof(passes[0]);

	if (GL_TEST_VTL_PASSES > 0 && (size_t)GL_TEST_VTL_PASSES < npass) {
		npass = GL_TEST_VTL_PASSES;
		pr_info("gl_test: vstexlod: passes 0..%u only (GL_TEST_VTL_PASSES)\n", (unsigned)npass - 1u);
	}
	for (size_t pass = 0; pass < npass; pass++) {
		uint32_t right = 0, dark_bad = 0, fx = 0, fy = 0, fgot = 0, fwant = 0;
		bool first = true;
		const float lod = passes[pass].lod;
		/* the level GL reads: the lod (clamped at 0) for a mipmap
		 * filter, else level 0 */
		const uint32_t level = passes[pass].mip && lod > 0.0f ? (uint32_t)lod : 0u;

		glActiveTexture(GL_TEXTURE0);
		glBindTexture(GL_TEXTURE_2D, passes[pass].control ? tex[2] : tex[0]);
		if (!passes[pass].control)
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER,
					passes[pass].mip ? GL_NEAREST_MIPMAP_NEAREST : GL_NEAREST);
		glUseProgram(passes[pass].konst ? progk : prog);
		if (!passes[pass].konst) {
			glUniform1f(l_lod, lod);
			glUniform1f(l_pick, passes[pass].fs ? 1.0f : 0.0f);
		}
		glClearColor(0, 0, 0, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_POINTS, 0, 64);
		glFinish();
		if (!check_gl_error("vstexlod draw")) {
			ok = false;
			break;
		}
		read_topdown();
		for (uint32_t ty = 0; ty < TH; ty++)
			for (uint32_t x = 0; x < TW; x++) {
				uint32_t gy = TH - 1u - ty, got = s_px[ty * TW + x];
				bool point = (x % 8u) == 4u && (gy % 8u) == 4u;
				uint32_t want = !point		   ? 0xff000000u
					      : passes[pass].konst ? vtl_texel(0, 8, 8)
					      : !passes[pass].fs   ? vtl_want(level, x / 8u, gy / 8u)
								   : vtl_b[0];

				if (chan_diff(got, want) <= 1) {
					right += point;
					continue;
				}
				dark_bad += !point;
				if (first) {
					first = false;
					fx = x;
					fy = gy;
					fgot = got;
					fwant = want;
				}
			}
		if (passes[pass].konst)
			pr_info("gl_test: vstexlod pass %u, constant coordinate and lod (one level, VS): %u of 64 "
			       "points right, %u pixels between them not black\n", (unsigned)pass, right, dark_bad);
		else if (passes[pass].control)
			pr_info("gl_test: vstexlod pass %u, the control (one level, VS): %u of 64 points right, %u "
			       "pixels between them not black\n", (unsigned)pass, right, dark_bad);
		else if (!passes[pass].fs)
			pr_info("gl_test: vstexlod pass %u, lod %d.%02d %s (VS, unit 0): %u of 64 points right, %u "
			       "pixels between them not black\n", (unsigned)pass, (int)lod,
			       (int)((lod < 0 ? -lod : lod) * 100.0f) % 100,
			       passes[pass].mip ? "mipmap" : "NEAREST (level 0)", right, dark_bad);
		else
			pr_info("gl_test: vstexlod pass %u, unit 1 (FS, the other texture): %u of 64 points right, %u "
			       "pixels between them not black\n", (unsigned)pass, right, dark_bad);
		if (!first) {
			uint32_t i = fx / 8u, j = fy / 8u;

			pr_info("gl_test: vstexlod: first wrong (%u, %u) from the bottom = 0x%08x, want 0x%08x\n", fx,
			       fy, fgot, fwant);
			pr_info("gl_test: vstexlod:   there unit 0 L0 0x%08x L1 0x%08x L2 0x%08x; unit 1 L0 0x%08x "
			       "L1 0x%08x L2 0x%08x\n", vtl_want(0, i, j), vtl_want(1, i, j), vtl_want(2, i, j),
			       vtl_b[0], vtl_b[1], vtl_b[2]);
		}
		ok &= gl_test_check(right == 64 && dark_bad == 0, right, 64);
	}
	/* the last pass, with every pass run: dEQP's own draw (the grid), the
	 * control texture on unit 0 */
	if (GL_TEST_VTL_PASSES == 0) {
		GLuint progg = link_program(gl_test_vtl_grid_vs_src, gl_test_vtl_fs_src);

		if (!progg) {
			ok = gl_test_check(false, 0, 1);
		} else {
			glUseProgram(progg);
			glUniform1i(glGetUniformLocation(progg, "u_vtex"), 0);
			glUniform1i(glGetUniformLocation(progg, "u_ftex"), 1);
			glUniform1f(glGetUniformLocation(progg, "u_pick"), 0.0f);
			glActiveTexture(GL_TEXTURE0);
			glBindTexture(GL_TEXTURE_2D, tex[2]);
			glDisableVertexAttribArray((GLuint)la);
			if (lak >= 0)
				glDisableVertexAttribArray((GLuint)lak);
			pr_info("gl_test: vstexlod pass %u: dEQP's grid\n", (unsigned)npass);
			ok &= vtl_grid_pass(progg);
			glUseProgram(0);
			glDeleteProgram(progg);
		}
	}
	glDisableVertexAttribArray((GLuint)la);
	if (lak >= 0)
		glDisableVertexAttribArray((GLuint)lak);
	glActiveTexture(GL_TEXTURE1);
	glBindTexture(GL_TEXTURE_2D, 0);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(3, tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	glDeleteProgram(progk);
	return ok ? 0 : -1;
}
#endif /* GL_TEST_V3D || GL_TEST_VIVANTE: case 129 */

#ifdef GL_TEST_V3D
/* ---- case 130: dEQP shaders.random.all_features.fragment.3 (S5) ------------
 *
 * S5 on the Pi 4 fails all_features.fragment.3, whose fragment shader is a
 * constant colour with no varyings, uniforms or samplers. v3d-m5 finds the IR
 * and our compiled code both give (1.0, 0.8125, 0.5625, 0.78125), RGBA8
 * (255, 207, 143, 199), on the host. The sources are rsg's for seed 3, byte
 * for byte (.claude/worktrees/s5_fragment3.{vert,frag,md}); the draw is
 * glsRandomShaderCase.cpp's: a 1x1 grid, dEQP_Position from a client array
 * (stride 0), glDrawElements of 6 client GL_UNSIGNED_SHORT indices, a 64x64
 * viewport, no clear before the draw. Three programs split the layers:
 * - rsg's pair as generated (the constant-folding expression);
 * - the same colour as a literal (the value, no expression);
 * - the same colour through a varying (a fragment shader WITH an input).
 * Each is drawn on the window (viewport 0,0) and on a 256x256 FBO at dEQP's
 * viewport for that size (119, 7), the FBO first cleared to a marker that
 * must survive outside the viewport. Printed per arm: every channel's
 * minimum and maximum against the expected value, and the first differing
 * pixel. Which channel, and which program, is off names the layer. V3D only. */





#define F3_WANT   0xc78fcfffu /* R 255, G 207, B 143, A 199 */
#define F3_MARKER 0xff402010u /* the FBO's clear: R 0x10, G 0x20, B 0x40 */
#define F3_FBO    256u
static uint32_t f3_px[F3_FBO * F3_FBO];

/* rsg's grid (glsRandomShaderCase.cpp:168-182, :214-219) */
static void f3_draw(GLuint prog)
{
	static const float pos[16] = { -1, 1, 0, 1, 1, 1, 0, 1, -1, -1, 0, 1, 1, -1, 0, 1 };
	static const uint16_t idx[6] = { 0, 2, 1, 1, 2, 3 };
	GLint lp = glGetAttribLocation(prog, "dEQP_Position");

	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, 0, pos);
	glEnableVertexAttribArray((GLuint)lp);
	glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, idx);
	glFlush();
	glDisableVertexAttribArray((GLuint)lp);
}

/* `n` pixels `px` against F3_WANT: the channels' min/max, the first
 * differing pixel (index into px, row width w, GL order). Alpha is judged
 * only when the bound framebuffer has alpha bits, as dEQP compares
 * (glsRandomShaderCase.cpp:529-535); its range is printed either way. */
static bool f3_judge(const char *what, const uint32_t *px, uint32_t n, uint32_t w)
{
	uint32_t lo[4] = { 255, 255, 255, 255 }, hi[4] = { 0 }, bad = 0, first = n;
	GLint abits = 0;
	uint32_t mask;

	glGetIntegerv(GL_ALPHA_BITS, &abits);
	mask = abits > 0 ? 0xffffffffu : 0x00ffffffu;

	for (uint32_t i = 0; i < n; i++) {
		for (int c = 0; c < 4; c++) {
			uint32_t v = (px[i] >> (8 * c)) & 0xffu;

			lo[c] = v < lo[c] ? v : lo[c];
			hi[c] = v > hi[c] ? v : hi[c];
		}
		if (chan_diff(px[i] & mask, F3_WANT & mask) > 1) {
			bad++;
			if (first == n)
				first = i;
		}
	}
	printk("gl_test: allfeat3 %s: %u of %u pixels differ; R %u..%u G %u..%u B %u..%u A %u..%u "
	       "(want 255 207 143 199; alpha %s)\n", what, bad, n, lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], lo[3],
	       hi[3], abits > 0 ? "judged" : "not judged, no alpha bits");
	if (bad)
		printk("gl_test: allfeat3 %s: first (%u, %u) from the viewport's bottom-left = 0x%08x\n", what,
		       first % w, first / w, px[first]);
	return gl_test_check(bad == 0, bad, 0);
}

static int case_allfeat3(void)
{
	static const struct {
		const char *name, *vs, *fs;
	} progs[] = {
		{ "rsg's pair", gl_test_f3_vs_src, gl_test_f3_fs_src },
		{ "the literal", gl_test_f3_vs_src, gl_test_f3_lit_fs_src },
		{ "through a varying", gl_test_f3_var_vs_src, gl_test_f3_var_fs_src },
	};
	GLuint tex, fbo;
	bool ok = true;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, F3_FBO, F3_FBO, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glBindTexture(GL_TEXTURE_2D, 0);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_SCISSOR_TEST);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	for (size_t p = 0; p < sizeof(progs) / sizeof(progs[0]); p++) {
		GLuint prog = link_program(progs[p].vs, progs[p].fs);
		char what[80];

		if (!prog) {
			printk("gl_test: allfeat3 %s: the program did not link\n", progs[p].name);
			ok = gl_test_check(false, 0, 1);
			continue;
		}
		/* the window: the viewport is all of it; cleared once to the
		 * marker so a pixel the draw missed shows */
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glViewport(0, 0, TW, TH);
		glClearColor(0x10 / 255.0f, 0x20 / 255.0f, 0x40 / 255.0f, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		f3_draw(prog);
		glFinish();
		ok &= check_gl_error("allfeat3 window");
		glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_rows);
		snprintf(what, sizeof(what), "%s, window", progs[p].name);
		ok &= f3_judge(what, s_rows, TW * TH, TW);
		/* the FBO at dEQP's viewport for 256x256 (119, 7), no clear
		 * before the draw beyond the marker */
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glViewport(0, 0, F3_FBO, F3_FBO);
		glClear(GL_COLOR_BUFFER_BIT);
		glViewport(119, 7, 64, 64);
		f3_draw(prog);
		glFinish();
		ok &= check_gl_error("allfeat3 fbo");
		glReadPixels(119, 7, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, s_rows);
		snprintf(what, sizeof(what), "%s, fbo at (119, 7)", progs[p].name);
		ok &= f3_judge(what, s_rows, 64 * 64, 64);
		/* outside the viewport: the marker, untouched */
		glReadPixels(0, 0, F3_FBO, F3_FBO, GL_RGBA, GL_UNSIGNED_BYTE, f3_px);
		{
			uint32_t touched = 0;

			for (uint32_t y = 0; y < F3_FBO; y++)
				for (uint32_t x = 0; x < F3_FBO; x++) {
					bool inside = x >= 119 && x < 183 && y >= 7 && y < 71;

					touched += !inside && chan_diff(f3_px[y * F3_FBO + x], F3_MARKER) > 1;
				}
			if (touched)
				printk("gl_test: allfeat3 %s: %u pixels outside the viewport changed\n", progs[p].name,
				       touched);
			ok &= gl_test_check(touched == 0, touched, 0);
		}
		glUseProgram(0);
		glDeleteProgram(prog);
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}
#endif

#ifdef GL_TEST_V3D
/* ---- case 131: dEQP shaders.random.all_features.fragment.1 (S5) ------------
 *
 * S5 on the Pi 4 (8d13b4df): the FIRST draw of all_features.fragment.1
 * faults at once, L2T, no churn before it, while 130 (fragment.3, no
 * sampler) passes. fragment.1's fragment shader samples a 2D texture at a
 * coordinate its constants fix, inversesqrt(vec2(k, m)) = (1.0, 0.5), and
 * has no varyings. The sources are rsg's for seed 1, byte for byte
 * (.claude/worktrees/s5_fragment1.{vert,frag,md}, v3d-m5); the texture and
 * the draw are glsRandomShaderCase.cpp's: 64x64 GL_RGBA/UNSIGNED_BYTE, one
 * level, fillWithComponentGradients((-1,-1,-1,2), (1,1,1,0)) in its old
 * style, CLAMP_TO_EDGE, LINEAR/LINEAR, unit 0 (:62-75, :300-322); the
 * sampler l = 0 and the int f = -19 (rsg's values for seed 1); a 1x1 grid,
 * glDrawElements of 6 client indices, then glFlush, no clear before. Three
 * programs split the layers:
 * - rsg's pair as generated;
 * - the same look-up at the literal (1.0, 0.5) (the value, no expression);
 * - a literal look-up at the centre (0.5, 0.5), off the clamped edge.
 * Each is drawn on the window and on a 256x256 FBO at dEQP's viewport for
 * that size (78, 15). The want is the bilinear sample of a C replica of the
 * texture (UNORM8 round to nearest even, as tcu stores it), within 3 per
 * channel; alpha judged only on a target with alpha bits. V3D only. */




#define F1_TEX 64u
static uint8_t f1_texels[F1_TEX * F1_TEX * 4];

/* tcu's float to UNORM8: clamped, x 255, rounded to nearest even */
static uint8_t f1_unorm8(float v)
{
	float x = (v < 0.0f ? 0.0f : v > 1.0f ? 1.0f : v) * 255.0f;
	uint32_t r = (uint32_t)x;
	float frac = x - (float)r;

	if (frac > 0.5f || (frac == 0.5f && (r & 1u)))
		r++;
	return (uint8_t)r;
}

/* tcu::fillWithComponentGradients, GRADIENT_STYLE_OLD (tcuTextureUtil.cpp
 * :1054-1068): min (-1, -1, -1, 2), max (1, 1, 1, 0) */
static void f1_fill(void)
{
	for (uint32_t y = 0; y < F1_TEX; y++)
		for (uint32_t x = 0; x < F1_TEX; x++) {
			float s = ((float)x + 0.5f) / (float)F1_TEX, t = ((float)y + 0.5f) / (float)F1_TEX;
			float c[4] = { -1.0f + 2.0f * ((s + t) * 0.5f), -1.0f + 2.0f * ((s + 1.0f - t) * 0.5f),
				       -1.0f + 2.0f * ((1.0f - s + t) * 0.5f), 2.0f - 2.0f * ((2.0f - s - t) * 0.5f) };

			for (int k = 0; k < 4; k++)
				f1_texels[(y * F1_TEX + x) * 4 + k] = f1_unorm8(c[k]);
		}
}

/* the bilinear, clamped sample at (s, t), as RGBA8 in a word */
static uint32_t f1_sample(float s, float t)
{
	float u = s * (float)F1_TEX - 0.5f, v = t * (float)F1_TEX - 0.5f;
	int i0 = (int)(u < 0.0f ? u - 1.0f : u), j0 = (int)(v < 0.0f ? v - 1.0f : v);
	float fu = u - (float)i0, fv = v - (float)j0;
	uint32_t out = 0;

	for (int k = 0; k < 4; k++) {
		float acc = 0.0f;

		for (int dj = 0; dj < 2; dj++)
			for (int di = 0; di < 2; di++) {
				int i = i0 + di, j = j0 + dj;

				i = i < 0 ? 0 : i > (int)F1_TEX - 1 ? (int)F1_TEX - 1 : i;
				j = j < 0 ? 0 : j > (int)F1_TEX - 1 ? (int)F1_TEX - 1 : j;
				acc += (di ? fu : 1.0f - fu) * (dj ? fv : 1.0f - fv) *
				       (float)f1_texels[((uint32_t)j * F1_TEX + (uint32_t)i) * 4 + (uint32_t)k];
			}
		out |= (uint32_t)(acc + 0.5f) << (8 * k);
	}
	return out;
}

/* `n` pixels against `want`, within 3 per channel; alpha only with alpha
 * bits. The channels' ranges and the first differing pixel printed. */
static bool f1_judge(const char *what, const uint32_t *px, uint32_t n, uint32_t w, uint32_t want)
{
	uint32_t lo[4] = { 255, 255, 255, 255 }, hi[4] = { 0 }, bad = 0, first = n;
	GLint abits = 0;
	uint32_t mask;

	glGetIntegerv(GL_ALPHA_BITS, &abits);
	mask = abits > 0 ? 0xffffffffu : 0x00ffffffu;
	for (uint32_t i = 0; i < n; i++) {
		for (int c = 0; c < 4; c++) {
			uint32_t v = (px[i] >> (8 * c)) & 0xffu;

			lo[c] = v < lo[c] ? v : lo[c];
			hi[c] = v > hi[c] ? v : hi[c];
		}
		if (chan_diff(px[i] & mask, want & mask) > 3) {
			bad++;
			if (first == n)
				first = i;
		}
	}
	printk("gl_test: allfeat1 %s: %u of %u pixels differ; R %u..%u G %u..%u B %u..%u A %u..%u "
	       "(want %u %u %u %u; alpha %s)\n", what, bad, n, lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], lo[3],
	       hi[3], want & 0xffu, (want >> 8) & 0xffu, (want >> 16) & 0xffu, want >> 24,
	       abits > 0 ? "judged" : "not judged, no alpha bits");
	if (bad)
		printk("gl_test: allfeat1 %s: first (%u, %u) from the viewport's bottom-left = 0x%08x\n", what,
		       first % w, first / w, px[first]);
	return gl_test_check(bad == 0, bad, 0);
}

static int case_allfeat1(void)
{
	static const struct {
		const char *name, *fs;
		float s, t;
	} progs[] = {
		{ "rsg's pair", gl_test_f1_fs_src, 1.0f, 0.5f },
		{ "the literal (1.0, 0.5)", gl_test_f1_edge_fs_src, 1.0f, 0.5f },
		{ "the centre (0.5, 0.5)", gl_test_f1_mid_fs_src, 0.5f, 0.5f },
	};
	GLuint tex, rt, fbo;
	bool ok = true;

	f1_fill();
	glActiveTexture(GL_TEXTURE0);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, F1_TEX, F1_TEX, 0, GL_RGBA, GL_UNSIGNED_BYTE, f1_texels);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
	glGenTextures(1, &rt);
	glBindTexture(GL_TEXTURE_2D, rt);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, F3_FBO, F3_FBO, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, rt, 0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glBindTexture(GL_TEXTURE_2D, tex);
	if (!check_gl_error("allfeat1 setup")) {
		glDeleteFramebuffers(1, &fbo);
		glDeleteTextures(1, &rt);
		glDeleteTextures(1, &tex);
		return -1;
	}
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_SCISSOR_TEST);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	for (size_t p = 0; p < sizeof(progs) / sizeof(progs[0]); p++) {
		GLuint prog = link_program(gl_test_f1_vs_src, progs[p].fs);
		uint32_t want = f1_sample(progs[p].s, progs[p].t);
		GLint lf;
		char what[80];

		if (!prog) {
			printk("gl_test: allfeat1 %s: the program did not link\n", progs[p].name);
			ok = gl_test_check(false, 0, 1);
			continue;
		}
		glUseProgram(prog);
		glUniform1i(glGetUniformLocation(prog, "l"), 0);
		lf = glGetUniformLocation(prog, "f");
		if (lf >= 0)
			glUniform1i(lf, -19);
		/* the window: all of it, cleared to the marker once */
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glViewport(0, 0, TW, TH);
		glClearColor(0x10 / 255.0f, 0x20 / 255.0f, 0x40 / 255.0f, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		f3_draw(prog);
		glFinish();
		ok &= check_gl_error("allfeat1 window");
		glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_rows);
		snprintf(what, sizeof(what), "%s, window", progs[p].name);
		ok &= f1_judge(what, s_rows, TW * TH, TW, want);
		/* the FBO at dEQP's viewport for 256x256 */
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glViewport(0, 0, F3_FBO, F3_FBO);
		glClear(GL_COLOR_BUFFER_BIT);
		glViewport(78, 15, 64, 64);
		f3_draw(prog);
		glFinish();
		ok &= check_gl_error("allfeat1 fbo");
		glReadPixels(78, 15, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, s_rows);
		snprintf(what, sizeof(what), "%s, fbo at (78, 15)", progs[p].name);
		ok &= f1_judge(what, s_rows, 64 * 64, 64, want);
		glUseProgram(0);
		glDeleteProgram(prog);
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDeleteFramebuffers(1, &fbo);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(1, &rt);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}
#endif

/* ---- case 108: uniform arrays read at a run-time index ---------------------
 *
 * dEQP-GLES2 indexing.uniform_array fails on the i.MX8MP in every dynamic and
 * loop read (24 cases), while gl_test 100 idxtex -- the same movar and
 * relative constant read -- passes there, and the host's lane interpreter
 * agrees with the IR on all of dEQP's shapes. What differs in our listings:
 * dEQP's read comes right after its movar and it indexes four times;
 * idxtex's read is three instructions after its one movar. So four
 * programs, each drawn over the whole target and read back at the centre:
 *
 *   A  uniidx_a.frag  dEQP's float_dynamic_read verbatim: four reads, each
 *                     right after its movar; 0.05+0.1+0.2+0.4 = 0.75 red
 *   B  uniidx_b.frag  one read of u_arr[2] right after its movar: 0.2 red
 *   C  uniidx_c.frag  the same read five instructions after its movar
 *   D  uniidx_d.frag  idxtex's fcols[fidx] in ES 1.00, no texture, the
 *                     read right after its movar: fcols[2]
 *
 * If a movar-to-read latency is the defect, B and D fail and C passes; if
 * re-indexing is, A fails and B passes; if all four pass, the defect is not
 * in the instruction stream. Every index reading as 0 gives A 0.2 and B/C
 * 0.05, so a wrong answer also says which element was read.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_uniidx(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const float arr[4] = { 0.05f, 0.1f, 0.2f, 0.4f };
	static const float fcols[4][4] = {
		{ 0.0f, 0.0f, 0.0f, 1.0f }, { 1.0f, 0.0f, 0.0f, 1.0f },
		{ 0.2f, 0.4f, 0.6f, 1.0f }, { 0.0f, 1.0f, 0.0f, 1.0f },
	};
	static const char *const ints[5] = { "ui_zero", "ui_one", "ui_two", "ui_three", "fidx" };
	static const int vals[5] = { 0, 1, 2, 3, 2 };
	const char *const fs[4] = {
		gl_test_uniidx_a_fs_src, gl_test_uniidx_b_fs_src, gl_test_uniidx_c_fs_src, gl_test_uniidx_d_fs_src,
	};
	const char *const what[4] = {
		"A: four reads, each right after its movar (sum 0.75)",
		"B: u_arr[2], read right after its movar (0.2)",
		"C: u_arr[2], read five instructions after its movar (0.2)",
		"D: vec4 fcols[fidx], read right after its movar",
	};
	const uint32_t want[4] = { 0xff0000bf, 0xff000033, 0xff000033, 0xff996633 };
	struct pc_vertex v[6];
	uint32_t fail_mask = 0;
	bool ok = true;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	for (int p = 0; p < 4; p++) {
		GLuint prog = link_program(gl_test_passthrough_vs_src, fs[p]);
		GLint loc;

		if (!prog)
			return -1;
		glUseProgram(prog);
		upload_vbo(v, sizeof(v));
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		for (int k = 0; k < 5; k++) {
			loc = glGetUniformLocation(prog, ints[k]);
			if (loc >= 0)
				glUniform1i(loc, vals[k]);
		}
		loc = glGetUniformLocation(prog, "u_arr");
		if (loc >= 0)
			glUniform1fv(loc, 4, arr);
		loc = glGetUniformLocation(prog, "fcols");
		if (loc >= 0)
			glUniform4fv(loc, 4, &fcols[0][0]);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(what[p]))
			return -1;
		read_topdown();
		{
			uint32_t got = s_px[(TH / 2) * TW + TW / 2];
			bool good = chan_diff(got, want[p]) <= 2;

			/* the log line only: the one recorded check is the mask below */
			pr_info("gl_test:   uniidx %s = 0x%08x  want 0x%08x  %s\n", what[p], got, want[p],
				good ? "ok" : "MISMATCH");
			if (!good)
				fail_mask |= 1u << p;
		}
	}
	/* One recorded check for all four programs, so a board that keeps only
	 * the first bad check still gets every answer: bit 0 A, 1 B, 2 C, 3 D.
	 * A = 0x1 alone is re-indexing; 0xb (A, B, D, not C) is a relative read
	 * whose destination is its movar's source register (A's `movar t0.x, t2`
	 * then `mov t2.x, u2[a.x]`, likewise B and D; C reads into another
	 * register five instructions later, as idxtex does); 0xf is a movar
	 * with no effect at all. */
	ok &= expect_int("uniidx fail mask (bit 0 A .. 3 D)", (long)fail_mask, 0);
	return ok ? 0 : -1;
}

/* ---- cases 116 / 117: which vertex stage lets 108 D's indexed read work ----
 *
 * 108 fails on the i.MX8MP in all four programs (mask 0xf at 2163f271):
 * every run-time-indexed uniform read under a passthrough vertex shader
 * reads element 0. gl_test 100 idxtex -- whose fragment stage compiles to
 * D's exact instructions, identical words at 151bdd7a and 2163f271 --
 * passes (2163f271). What idxtex has and 108 lacks is in the vertex stage:
 * uniforms (so PS_UNIFORM_BASE > 0), a run-time-indexed read of its own,
 * and a texture. 116 pairs D's FS with a VS that has uniforms and its own
 * indexed read; 117 with a VS that has one uniform and no index. Each draws
 * full screen, blue clear, D's fcols[2] = 0xff996633 wanted at the centre:
 *   116 pass, 117 pass  -> a vertex stage with uniforms is enough
 *   116 pass, 117 fail  -> it takes a relative read in the vertex stage
 *   both fail           -> the texture (or something else of idxtex's)
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_uniidx_vs(int which)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const float fcols[4][4] = {
		{ 0.0f, 0.0f, 0.0f, 1.0f }, { 1.0f, 0.0f, 0.0f, 1.0f },
		{ 0.2f, 0.4f, 0.6f, 1.0f }, { 0.0f, 1.0f, 0.0f, 1.0f },
	};
	static const float vcols[4][4] = {
		{ 0.1f, 0.1f, 0.1f, 1.0f }, { 0.2f, 0.2f, 0.2f, 1.0f },
		{ 0.3f, 0.3f, 0.3f, 1.0f }, { 0.4f, 0.4f, 0.4f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint loc;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	prog = link_program(which == 1 ? gl_test_uniidx_e_vs_src : gl_test_uniidx_f_vs_src,
			    gl_test_uniidx_d_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glEnableVertexAttribArray(0);
	loc = glGetUniformLocation(prog, "fcols");
	if (loc >= 0)
		glUniform4fv(loc, 4, &fcols[0][0]);
	loc = glGetUniformLocation(prog, "fidx");
	if (loc >= 0)
		glUniform1i(loc, 2);
	loc = glGetUniformLocation(prog, "vcols");
	if (loc >= 0)
		glUniform4fv(loc, 4, &vcols[0][0]);
	loc = glGetUniformLocation(prog, "vidx");
	if (loc >= 0)
		glUniform1i(loc, 3);
	loc = glGetUniformLocation(prog, "u_k");
	if (loc >= 0)
		glUniform4f(loc, 0.5f, 0.5f, 0.5f, 1.0f);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error(which == 1 ? "uniidxe" : "uniidxf"))
		return -1;
	read_topdown();
	return expect_px(which == 1 ? "uniidxe: D's fcols[fidx], VS indexes too"
				    : "uniidxf: D's fcols[fidx], VS has a uniform",
			 TW / 2, TH / 2, 0xff996633, 2) ? 0 : -1;
}

/* ---- cases 120 / 121 / 122: where a texld must sit for 108 D to work -----
 *
 * 116 and 117 both read element 0 on the i.MX8MP: idxtex's vertex stage is
 * not what makes it pass. What is left is its fragment stage's texld, which
 * the GC7000 back end selects between the movar and the relative read. These
 * three add one texture fetch to D's fcols[fidx] and move it (listings at the
 * online compiler of this commit, gpu-vivante):
 *   120 G  movar, texld, mov t2, u5[a.x]   (idxtex's order)
 *   121 H  movar, mov t1, u5[a.x], texld   (the texld needs the read's value)
 *   122 I  texld, ..., movar, mov u5[a.x]  (the index needs the texel)
 * The texture is 1x1 opaque black, so the colour is 0.5 * (fcols[2] +
 * (0, 0, 0, 1)) = (0.1, 0.2, 0.3, 1.0); element 0 gives (0, 0, 0, 1).
 *   120 pass, 121 and 122 fail -> the read needs a texld after its movar
 *   all three fail             -> a texld is not it either
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_uniidx_tex(int which)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const float fcols[4][4] = {
		{ 0.0f, 0.0f, 0.0f, 1.0f }, { 1.0f, 0.0f, 0.0f, 1.0f },
		{ 0.2f, 0.4f, 0.6f, 1.0f }, { 0.0f, 1.0f, 0.0f, 1.0f },
	};
	static const uint8_t texel[4] = { 0, 0, 0, 255 };
	static const char *const name[3] = { "uniidxg", "uniidxh", "uniidxi" };
	static const char *const what[3] = {
		"uniidxg: fcols[fidx], texld between the movar and the read",
		"uniidxh: fcols[fidx], texld after the read",
		"uniidxi: fcols[fidx], texld before the movar",
	};
	const char *const fs[3] = { gl_test_uniidx_g_fs_src, gl_test_uniidx_h_fs_src, gl_test_uniidx_i_fs_src };
	struct pc_vertex v[6];
	GLuint prog, tex = 0;
	GLint loc;
	bool ok;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	prog = link_program(gl_test_passthrough_vs_src, fs[which]);
	if (!prog)
		return -1;
	glGenTextures(1, &tex);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, texel);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	loc = glGetUniformLocation(prog, "fcols");
	if (loc >= 0)
		glUniform4fv(loc, 4, &fcols[0][0]);
	loc = glGetUniformLocation(prog, "fidx");
	if (loc >= 0)
		glUniform1i(loc, 2);
	loc = glGetUniformLocation(prog, "ftex");
	if (loc >= 0)
		glUniform1i(loc, 0);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error(name[which]))
		return -1;
	read_topdown();
	ok = expect_px(what[which], TW / 2, TH / 2, rgba_u32(26, 51, 77, 255), 2);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error(name[which]) ? 0 : -1;
}

/* ---- case 132: GL_LUMINANCE, GL_ALPHA, GL_LUMINANCE_ALPHA sampled -----------
 * A 4x4 texture of each, one distinct value per texel (L = 16 i + 8, A =
 * 255 - L), NEAREST over the whole 64x64 target, every pixel judged against
 * what ES 2.0 table 3.12 says it samples as: (L,L,L,1), (0,0,0,A), (L,L,L,A).
 * Then an 8x8 L8 chain with a distinct value per level, drawn into a 4x4
 * viewport with NEAREST_MIPMAP_NEAREST so level 1 (200) is the one sampled:
 * the narrow store's level offsets. On the PowerVR the case runs every arm of
 * pvr_narrow_la itself -- 0 (expanded into RGBA8), 1 (narrow, pack info
 * 0x140), 2 (narrow, 0x11f) -- a texture keeping the layout it was created
 * with; on the GC7000 with the test knobs both arms of gc_debug_narrow_la
 * (0 expanded, 1 the unit's L8/A8/A8L8); elsewhere, and on a GC7000 build
 * without them, the one arm there is. One check: a fail
 * mask, four bits an arm (L8, A8, LA8, mip). */
#if !defined(GL_TEST_VIVANTE) && !defined(GL_TEST_V3D)
extern int pvr_narrow_la(int mode);
#define NARROWLA_ARMS 3
#elif defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
/* the GC7000's opt-in (vivante/backend.rs gc_debug_narrow_la, a test knob):
 * arm 0 expanded into RGBA8, arm 1 narrow with the unit's L8 / A8 / A8L8 */
extern int gc_debug_set_narrow_la(int on);
#define NARROWLA_ARMS 2
#else
#define NARROWLA_ARMS 1
#endif

static uint8_t narrowla_l(int tx, int ty)
{
	return (uint8_t)(16 * (ty * 4 + tx) + 8);
}

static unsigned narrowla_draw(GLenum fmt, const char *name)
{
	uint8_t img[4 * 4 * 2];
	unsigned bad = 0, first = 0, got_first = 0, want_first = 0;
	GLuint tex;

	for (int ty = 0; ty < 4; ty++) {
		for (int tx = 0; tx < 4; tx++) {
			uint8_t l = narrowla_l(tx, ty), a = (uint8_t)(255 - l);
			int i = ty * 4 + tx;

			if (fmt == GL_LUMINANCE_ALPHA) {
				img[i * 2] = l;
				img[i * 2 + 1] = a;
			} else {
				img[i] = fmt == GL_ALPHA ? a : l;
			}
		}
	}
	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, (GLint)fmt, 4, 4, 0, fmt, GL_UNSIGNED_BYTE, img);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	wrap_quad(1.0f, 1.0f);
	glViewport(0, 0, TW, TH);
	glClearColor(0.5f, 0.5f, 0.5f, 0.5f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error(name)) {
		glDeleteTextures(1, &tex);
		return TW * TH;
	}
	read_topdown();
	for (int r = 0; r < (int)TH; r++) {
		for (int x = 0; x < (int)TW; x++) {
			int gy = (int)TH - 1 - r; /* GL row: v grows upwards */
			uint8_t l = narrowla_l(x * 4 / TW, gy * 4 / TH), a = (uint8_t)(255 - l);
			uint32_t want = fmt == GL_LUMINANCE ? rgba_u32(l, l, l, 255)
				      : fmt == GL_ALPHA     ? rgba_u32(0, 0, 0, a)
							    : rgba_u32(l, l, l, a);
			uint32_t got = s_px[r * TW + x];

			if (chan_diff(got, want) > 1) {
				if (bad++ == 0) {
					first = (unsigned)(r * TW + x);
					got_first = got;
					want_first = want;
				}
			}
		}
	}
	pr_info("gl_test:   narrowla %-6s %u of %u pixels wrong", name, bad, TW * TH);
	if (bad)
		pr_info(", first top-down (%u,%u) = 0x%08x want 0x%08x", first % TW, first / TW, got_first,
			want_first);
	pr_info("\n");
	glDeleteTextures(1, &tex);
	return bad;
}

static unsigned narrowla_mip(void)
{
	static const uint8_t lv[4] = { 10, 200, 60, 90 };
	uint8_t img[8 * 8];
	unsigned bad = 0;
	GLuint tex;

	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	for (int l = 0, side = 8; l < 4; l++, side /= 2) {
		memset(img, lv[l], sizeof(img));
		glTexImage2D(GL_TEXTURE_2D, l, GL_LUMINANCE, side, side, 0, GL_LUMINANCE, GL_UNSIGNED_BYTE, img);
	}
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	wrap_quad(1.0f, 1.0f);
	glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glViewport(0, 0, 4, 4); /* 8x8 onto 4x4: lod 1 */
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	glViewport(0, 0, TW, TH);
	if (!check_gl_error("narrowla mip")) {
		glDeleteTextures(1, &tex);
		return 16;
	}
	read_topdown();
	for (int r = (int)TH - 4; r < (int)TH; r++)
		for (int x = 0; x < 4; x++)
			bad += chan_diff(s_px[r * TW + x], rgba_u32(200, 200, 200, 255)) > 1;
	pr_info("gl_test:   narrowla mip    %u of 16 pixels not level 1 (0x%08x at the corner)\n", bad,
		s_px[(TH - 1) * TW]);
	glDeleteTextures(1, &tex);
	return bad;
}

static int case_narrowla(void)
{
#ifdef GL_TEST_VIVANTE
	static const char *const arm_name[3] = { "expanded into RGBA8", "narrow, TEXTURE_FORMAT L8/A8/A8L8",
						 "" };
#else
	static const char *const arm_name[3] = { "expanded into RGBA8", "narrow, pack info 0x140",
						 "narrow, pack info 0x11f" };
#endif
	unsigned mask = 0;
	GLuint prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);

	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int arm = 0; arm < NARROWLA_ARMS; arm++) {
#if !defined(GL_TEST_VIVANTE) && !defined(GL_TEST_V3D)
		int was = pvr_narrow_la(arm);
#elif defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
		int was = gc_debug_set_narrow_la(arm);
#endif
		pr_info("gl_test: narrowla arm %d (%s)\n", arm, NARROWLA_ARMS > 1 ? arm_name[arm] : "this GPU's store");
		mask |= (narrowla_draw(GL_LUMINANCE, "L8") ? 1u : 0u) << (arm * 4);
		mask |= (narrowla_draw(GL_ALPHA, "A8") ? 2u : 0u) << (arm * 4);
		mask |= (narrowla_draw(GL_LUMINANCE_ALPHA, "LA8") ? 4u : 0u) << (arm * 4);
		mask |= (narrowla_mip() ? 8u : 0u) << (arm * 4);
#if !defined(GL_TEST_VIVANTE) && !defined(GL_TEST_V3D)
		pvr_narrow_la(was);
#elif defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
		gc_debug_set_narrow_la(was);
#endif
	}
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	return expect_int("narrowla fail mask", (long)mask, 0) ? 0 : -1;
}

/* ---- case 127: where a line's varyings are interpolated -------------------
 *
 * dEQP-GLES2 rasterization.interpolation basic.lines, line_strip,
 * line_strip_wide and projected.lines(_wide) fail on the i.MX8MP while
 * Mesa's etnaviv passes them on this GPU (its GC7000 CI lists fail none), and
 * our line state is Mesa's register for register (WIDE_LINE, width / 2,
 * SE_CONFIG without LAST_PIXEL, PA_SYSTEM_MODE half-pixel centre + last
 * provoking vertex). This fingerprints the interpolation instead of the
 * state: two red-to-green lines with fractional endpoints, 15.75 pixels long,
 * a horizontal one at window y 20.5 and an x-major diagonal, every covered
 * pixel's green share t = G / (R + G) fitted against three models:
 *   M0 dEQP's reference, t = (x + 0.5 - x0) / (x1 - x0) along the major axis
 *   M1 the covered pixels' own span, t = (x - first) / (last - first)
 *   M2 the pixel corner, t = (x - x0) / (x1 - x0)
 * (M0 and M2 are 0.5 pixel, ~8 LSB, apart everywhere; M0 and M1 ~11 LSB at
 * the far end, the span being asymmetric). The one recorded check
 * is the fit mask: bits 0-2 the horizontal line's models within 2.5 LSB,
 * bits 3-5 the diagonal's; dEQP's model is 0x9. The log carries every pixel
 * and the covered span (dEQP's diamond exit covers columns 10..25).
 * ------------------------------------------------------------------------- */
static unsigned line_fit(int line, float x0, float x1, const int *cov_y)
{
	int first = -1, last = -1;
	float err[3] = { 0, 0, 0 };
	unsigned fit = 0;

	for (int x = 0; x < (int)TW; x++) {
		if (cov_y[x] < 0)
			continue;
		if (first < 0)
			first = x;
		last = x;
	}
	if (first < 0 || last == first) {
		pr_info("gl_test:   lines %d: %s\n", line, first < 0 ? "no pixel drawn" : "one pixel drawn");
		return 0;
	}
	for (int x = first; x <= last; x++) {
		float model[3], t;
		uint32_t px;
		int r, g;

		if (cov_y[x] < 0)
			continue;
		px = s_px[cov_y[x] * TW + x];
		r = (int)(px & 0xff);
		g = (int)((px >> 8) & 0xff);
		t = (r + g) ? (float)g / (float)(r + g) : 0.0f;
		model[0] = ((float)x + 0.5f - x0) / (x1 - x0);
		model[1] = (float)(x - first) / (float)(last - first);
		model[2] = ((float)x - x0) / (x1 - x0);
		for (int m = 0; m < 3; m++) {
			float e = (t - model[m]) * 255.0f;

			if (e < 0)
				e = -e;
			if (e > err[m])
				err[m] = e;
		}
		pr_info("gl_test:   lines %d x %2d row %2d: r %3d g %3d t %.3f  M0 %.3f M1 %.3f M2 %.3f\n", line, x,
			cov_y[x], r, g, (double)t, (double)model[0], (double)model[1], (double)model[2]);
	}
	for (int m = 0; m < 3; m++)
		if (err[m] <= 2.5f)
			fit |= 1u << m;
	pr_info("gl_test:   lines %d: covered %d..%d, worst LSB M0 %.1f M1 %.1f M2 %.1f -> fit 0x%x\n", line, first,
		last, (double)err[0], (double)err[1], (double)err[2], fit);
	return fit;
}

static int case_lines(void)
{
	/* window coordinates of the two lines, y up */
	static const float lw[2][4] = {
		{ 10.45f, 20.5f, 26.2f, 20.5f },
		{ 10.45f, 40.2f, 26.2f, 48.1f },
	};
	struct pc_vertex v[4];
	int cov[2][TW];
	unsigned fit;
	GLuint prog;

	for (int l = 0; l < 2; l++) {
		for (int e = 0; e < 2; e++) {
			struct pc_vertex *p = &v[l * 2 + e];

			p->pos[0] = lw[l][e * 2] / (float)TW * 2.0f - 1.0f;
			p->pos[1] = lw[l][e * 2 + 1] / (float)TH * 2.0f - 1.0f;
			p->pos[2] = 0.0f;
			p->pos[3] = 1.0f;
			p->color[0] = e ? 0.0f : 1.0f;
			p->color[1] = e ? 1.0f : 0.0f;
			p->color[2] = 0.0f;
			p->color[3] = 1.0f;
		}
	}
	prog = link_program(gl_test_passthrough_vs_src, gl_test_interp_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);
	glLineWidth(1.0f);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_LINES, 0, 4);
	glFinish();
	if (!check_gl_error("lines"))
		return -1;
	read_topdown();
	/* per column, the covered row of each line (top-down index), -1 for none:
	 * the horizontal line's own row, the diagonal's brightest in its band */
	for (int x = 0; x < (int)TW; x++) {
		int best = -1, best_sum = 0;
		uint32_t px = s_px[(TH - 1 - 20) * TW + x];

		cov[0][x] = ((px & 0xff) + ((px >> 8) & 0xff)) > 64 ? (int)(TH - 1 - 20) : -1;
		for (int wy = 36; wy <= 52; wy++) {
			uint32_t q = s_px[(TH - 1 - (uint32_t)wy) * TW + x];
			int sum = (int)(q & 0xff) + (int)((q >> 8) & 0xff);

			if (sum > 64 && sum > best_sum) {
				best_sum = sum;
				best = (int)(TH - 1) - wy;
			}
		}
		cov[1][x] = best;
	}
	fit = line_fit(0, lw[0][0], lw[0][2], cov[0]) | line_fit(1, lw[1][0], lw[1][2], cov[1]) << 3;
	return expect_int("lines fit (0-2 M0/M1/M2 horizontal, 3-5 diagonal)", (long)fit, 0x9) ? 0 : -1;
}

/* ---- case 164: deleting the buffer an attribute reads (review R-24) --------
 *
 * ES 2.0.25 2.9: "If a buffer object is deleted while it is bound, all
 * bindings to that object in the current context (i.e. in the thread that
 * called DeleteBuffers) are reset to zero." A vertex array's buffer binding
 * (VERTEX_ATTRIB_ARRAY_BUFFER_BINDING) is one of them. The attribute kept the
 * deleted name until 2026-09-29.
 *   1. the left half drawn green from a VBO (the control: the program and the
 *      array work);
 *   2. the VBO deleted: attribute 0's buffer binding must read 0;
 *   3. a new buffer holding a full-screen quad, bound to ARRAY_BUFFER only,
 *      then a red draw. Attribute 0 is now a client array whose pointer is
 *      the old offset 0, so YakoGL refuses the draw by name
 *      (GL_INVALID_OPERATION) and the frame keeps green on black. With the
 *      defect, and glGenBuffers handing back the deleted name, the draw read
 *      the new buffer and the whole frame turned red. The line says whether
 *      the name came back: if it did not, step 3 cannot tell the two apart.
 * ------------------------------------------------------------------------- */
static int case_bufdel(void)
{
	static const float left[6][2] = {
		{ -1.0f, -1.0f }, { 0.0f, -1.0f }, { -1.0f, 1.0f },
		{  0.0f, -1.0f }, { 0.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const float full[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog, vbo, again;
	GLint loc, binding = -1;
	GLenum err;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "u_color");
	if (loc < 0) {
		pr_err("gl_test: bufdel: u_color location not found\n");
		return -1;
	}
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = left[i][0]; v[i].pos[1] = left[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	vbo = upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glUniform4f(loc, 0.0f, 1.0f, 0.0f, 1.0f);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("bufdel control draw"))
		return -1;
	read_topdown();
	ok &= expect_px("control: green left half", 16, 32, 0xff00ff00, 2);
	ok &= expect_px("control: black right half", 48, 32, 0xff000000, 2);

	glDeleteBuffers(1, &vbo);
	glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &binding);
	ok &= expect_int("attrib 0 buffer binding after delete", binding, 0);

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = full[i][0]; v[i].pos[1] = full[i][1];
	}
	again = upload_vbo(v, sizeof(v));
	pr_info("gl_test:   bufdel: deleted name %u, new buffer %u (%s)\n", (unsigned)vbo, (unsigned)again,
		again == vbo ? "reused: step 3 discriminates" : "NOT reused: step 3 cannot tell");
	glUniform4f(loc, 1.0f, 0.0f, 0.0f, 1.0f);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	err = glGetError();
	ok &= expect_int("draw with a zero binding at offset 0", (long)err, GL_INVALID_OPERATION);
	glFinish();
	read_topdown();
	ok &= expect_px("still green left half", 16, 32, 0xff00ff00, 2);
	ok &= expect_px("still black right half", 48, 32, 0xff000000, 2);

	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &again);
	if (!check_gl_error("bufdel teardown"))
		return -1;
	return ok ? 0 : -1;
}

/* ---- case 165: a pending clear stays with its context (review R-04) --------
 *
 * glClear only records the clear; the next draw or flush of the context
 * carries it out. Until 2026-09-29 that record was one for the process, and
 * the C API's gl_context_set_current did not flush it: context B's next draw
 * took context A's clear (A's colour in B's surface) and A lost it. EGL 1.5
 * 3.7.3: switching the current context flushes the one left.
 *   1. A (the harness context) clears red; nothing is drawn yet;
 *   2. gl_context_create makes B current; B draws a green left half, no clear:
 *      B's right half must not be red (the defect), its left half green;
 *   3. B clears blue; gl_context_set_current(A): A reads red everywhere (its
 *      clear was not lost);
 *   4. back in B: B reads blue everywhere (its clear was carried out).
 * SKIP when the pool has no free slot for B.
 * ------------------------------------------------------------------------- */
static int case_ctxclear(void)
{
	static const float left[6][2] = {
		{ -1.0f, -1.0f }, { 0.0f, -1.0f }, { -1.0f, 1.0f },
		{  0.0f, -1.0f }, { 0.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct gl_context *a = s_harness_ctx, *b;
	struct pc_vertex v[6];
	GLuint prog;
	GLint loc;
	bool ok = true;

	if (!a) {
		pr_err("gl_test: ctxclear: no harness context\n");
		return -1;
	}
	gl_context_set_current(a);
	glViewport(0, 0, TW, TH);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	if (!check_gl_error("ctxclear A clear"))
		return -1;

	b = gl_context_create(TW, TH);
	if (!b) {
		gl_context_set_current(a);
		pr_info("gl_test: case 165 ctxclear: SKIP (no free context slot for B)\n");
		return -3;
	}
	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog) {
		gl_context_destroy(b);
		gl_context_set_current(a);
		return -1;
	}
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "u_color");
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = left[i][0]; v[i].pos[1] = left[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glEnableVertexAttribArray(0);
	glViewport(0, 0, TW, TH);
	glUniform4f(loc, 0.0f, 1.0f, 0.0f, 1.0f);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	ok &= check_gl_error("ctxclear B draw");
	read_topdown();
	ok &= expect_px("B: green left half", 16, 32, 0xff00ff00, 2);
	pr_info("gl_test:   ctxclear: B's right half 0x%08x (A's red there is the defect)\n", s_px[32 * TW + 48]);
	ok &= expect_int("B: right half is not A's red", chan_diff(s_px[32 * TW + 48], 0xff0000ff) > 2, 1);

	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	gl_context_set_current(a);
	glFinish();
	read_topdown();
	ok &= expect_px("A: red (its clear kept)", 48, 32, 0xff0000ff, 2);
	ok &= expect_px("A: red corner", 1, 1, 0xff0000ff, 2);

	gl_context_set_current(b);
	glFinish();
	read_topdown();
	ok &= expect_px("B: blue (its clear carried out)", 48, 32, 0xffff0000, 2);
	ok &= expect_px("B: blue left half", 16, 32, 0xffff0000, 2);

	gl_context_set_current(a);
	gl_context_destroy(b);
	gl_context_set_current(a);
	if (!check_gl_error("ctxclear teardown"))
		return -1;
	return ok ? 0 : -1;
}

/* ---- case 106: three draw buffers (MRT step 3) ------------------------------
 *
 * GL_MAX_DRAW_BUFFERS is every colour attachment (3) on every GPU here; the
 * case SKIPs below 3. Three RGBA8 attachments, glDrawBuffers of all three:
 *   1. a clear to grey reaches all three (the clear quad writes every target);
 *   2. one draw writes red, green, blue to attachments 0, 1, 2 (each output
 *      at its own location, PS_OUTPUT_REG byte n);
 *   3. glDrawBuffers(ATT0, NONE, ATT2), a clear to black read back BEFORE the
 *      draw (0 and 2 black, 1 still green), then a draw of the same program:
 *      0 and 2 red and blue again, 1 still green. Without the read-back
 *      between (until 2026-09-26) a clear and a draw that reached nothing
 *      passed too, since part 2 had left exactly those colours: bits 6 and 8
 *      measured nothing, and a GL core that drops every target after the
 *      NONE slot (draw.rs resolve_targets) was invisible.
 * Each attachment is read with glReadBuffer at the centre. The one recorded
 * check is a fail mask: bits 0-2 the clear, 3-5 the draw, 6-8 the NONE draw,
 * 9-11 the NONE clear, bit n of each group attachment n. Case 55 (es3mrt) is the PowerVR's two-
 * attachment case and never calls glDrawBuffers; this one is the GC7000's.
 * ------------------------------------------------------------------------- */
static uint32_t mrt3_read(int n)
{
	uint8_t px[4] = { 0, 0, 0, 0 };

	glReadBuffer(GL_COLOR_ATTACHMENT0 + (GLenum)n);
	glReadPixels(TW / 2, TH / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	return rgba_u32(px[0], px[1], px[2], px[3]);
}

static int case_mrt3(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const GLenum all[3] = { GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2 };
	static const GLenum hole[3] = { GL_COLOR_ATTACHMENT0, GL_NONE, GL_COLOR_ATTACHMENT2 };
	const uint32_t colour[3] = { rgba_u32(255, 0, 0, 255), rgba_u32(0, 255, 0, 255), rgba_u32(0, 0, 255, 255) };
	const uint32_t grey = rgba_u32(128, 128, 128, 255);
	GLuint prog, fbo = 0, tex[3] = { 0, 0, 0 };
	GLint maxdb = 0;
	uint32_t fail = 0;

	glGetIntegerv(GL_MAX_DRAW_BUFFERS, &maxdb);
	if (maxdb < 3) {
		pr_info("gl_test: case 106 mrt3: SKIP (GL_MAX_DRAW_BUFFERS %d < 3)\n", (int)maxdb);
		return -3;
	}
	free_all_objects();
	prog = link_program(gl_test_es300_mrt3_vs_src, gl_test_es300_mrt3_fs_src);
	if (!prog)
		return -1;
	glGenTextures(3, tex);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	for (int n = 0; n < 3; n++) {
		glBindTexture(GL_TEXTURE_2D, tex[n]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glFramebufferTexture2D(GL_FRAMEBUFFER, all[n], GL_TEXTURE_2D, tex[n], 0);
	}
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: mrt3: framebuffer incomplete\n");
		return -1;
	}
	glDrawBuffers(3, all);
	if (!check_gl_error("mrt3 setup"))
		return -1;
	glUseProgram(prog);
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);

	glClearColor(128.0f / 255.0f, 128.0f / 255.0f, 128.0f / 255.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	for (int n = 0; n < 3; n++) {
		uint32_t got = mrt3_read(n);

		pr_info("gl_test:   mrt3 clear   attachment %d = 0x%08x  want 0x%08x\n", n, got, grey);
		if (chan_diff(got, grey) > 2)
			fail |= 1u << n;
	}

	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	for (int n = 0; n < 3; n++) {
		uint32_t got = mrt3_read(n);

		pr_info("gl_test:   mrt3 draw    attachment %d = 0x%08x  want 0x%08x\n", n, got, colour[n]);
		if (chan_diff(got, colour[n]) > 2)
			fail |= 1u << (3 + n);
	}

	/* attachment 1 keeps the draw's green; 0 and 2 are cleared black, read
	 * back as black here, then drawn red and blue again */
	glDrawBuffers(3, hole);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	for (int n = 0; n < 3; n++) {
		const uint32_t want = n == 1 ? colour[1] : rgba_u32(0, 0, 0, 255);
		uint32_t got = mrt3_read(n);

		pr_info("gl_test:   mrt3 hole clear attachment %d = 0x%08x  want 0x%08x\n", n, got, want);
		if (chan_diff(got, want) > 2)
			fail |= 1u << (9 + n);
	}
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	for (int n = 0; n < 3; n++) {
		uint32_t got = mrt3_read(n);

		pr_info("gl_test:   mrt3 hole draw  attachment %d = 0x%08x  want 0x%08x\n",
			n, got, colour[n]);
		if (chan_diff(got, colour[n]) > 2)
			fail |= 1u << (6 + n);
	}

	glReadBuffer(GL_COLOR_ATTACHMENT0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(3, tex);
	if (!check_gl_error("mrt3 teardown"))
		return -1;
	return expect_int("mrt3 fail mask (0-2 clear, 3-5 draw, 6-8 NONE draw, 9-11 NONE clear)", (long)fail, 0) ? 0 : -1;
}

/* ---- case 163: draw buffer 0 GL_NONE, draw buffer 1 written (review R-09) --
 *
 * ES 3.0 4.2.1: "Specifying NONE as the draw buffer for a fragment color will
 * inhibit that fragment color from being written." Two RGBA8 attachments,
 * both cleared grey and read back (the control), then
 * glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) and one draw of case 106's
 * program (outputs red, green, blue). Until 2026-09-29 output 0 went to
 * attachment 0 whatever draw buffer 0 said: attachment 0 turned red.
 *   attachment 0 must stay grey, always;
 *   no GL error: attachment 1 must be green (the GPU wrote slot 1 alone);
 *   GL_INVALID_OPERATION: the backend refused the draw by name (it always
 *   stores slot 0; the PowerVR, V3D and GC7000 all do today) and attachment 1
 *   must stay grey.
 * Any other error fails. SKIP on an ES 2.0 library (glDrawBuffers is ES 3.0)
 * and below GL_MAX_DRAW_BUFFERS 2.
 * ------------------------------------------------------------------------- */
static int case_drawbuf0(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const GLenum both[2] = { GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1 };
	static const GLenum hole0[2] = { GL_NONE, GL_COLOR_ATTACHMENT1 };
	const uint32_t grey = rgba_u32(128, 128, 128, 255);
	const uint32_t green = rgba_u32(0, 255, 0, 255);
	const char *ver = (const char *)glGetString(GL_VERSION);
	GLuint prog, fbo = 0, tex[2] = { 0, 0 };
	GLint maxdb = 0;
	GLenum err;
	uint32_t got0, got1;
	bool ok = true;

	if (!ver || strncmp(ver, "OpenGL ES 2.", 12) == 0) {
		pr_info("gl_test: case 163 drawbuf0: SKIP (an ES 2.0 library has no glDrawBuffers)\n");
		return -3;
	}
	glGetIntegerv(GL_MAX_DRAW_BUFFERS, &maxdb);
	if (maxdb < 2) {
		pr_info("gl_test: case 163 drawbuf0: SKIP (GL_MAX_DRAW_BUFFERS %d < 2)\n", (int)maxdb);
		return -3;
	}
	free_all_objects();
	prog = link_program(gl_test_es300_mrt3_vs_src, gl_test_es300_mrt3_fs_src);
	if (!prog)
		return -1;
	glGenTextures(2, tex);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	for (int n = 0; n < 2; n++) {
		glBindTexture(GL_TEXTURE_2D, tex[n]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glFramebufferTexture2D(GL_FRAMEBUFFER, both[n], GL_TEXTURE_2D, tex[n], 0);
	}
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: drawbuf0: framebuffer incomplete\n");
		return -1;
	}
	glDrawBuffers(2, both);
	glUseProgram(prog);
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);
	glClearColor(128.0f / 255.0f, 128.0f / 255.0f, 128.0f / 255.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("drawbuf0 setup"))
		return -1;
	got0 = mrt3_read(0);
	got1 = mrt3_read(1);
	pr_info("gl_test:   drawbuf0 control attachment 0 = 0x%08x, 1 = 0x%08x  want 0x%08x\n", got0, got1, grey);
	ok &= expect_int("control: both attachments grey", chan_diff(got0, grey) <= 2 && chan_diff(got1, grey) <= 2, 1);

	glDrawBuffers(2, hole0);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	err = glGetError();
	glFinish();
	got0 = mrt3_read(0);
	got1 = mrt3_read(1);
	pr_info("gl_test:   drawbuf0 {NONE, ATT1}: error 0x%x (%s), attachment 0 = 0x%08x, 1 = 0x%08x\n",
		(unsigned)err, err == GL_NO_ERROR ? "drawn" : err == GL_INVALID_OPERATION ? "refused by the backend" : "?",
		got0, got1);
	ok &= expect_int("attachment 0 keeps its grey", chan_diff(got0, grey) <= 2, 1);
	ok &= expect_int("no error, or the backend's refusal", err == GL_NO_ERROR || err == GL_INVALID_OPERATION, 1);
	if (err == GL_NO_ERROR)
		ok &= expect_int("drawn: attachment 1 green", chan_diff(got1, green) <= 2, 1);
	else
		ok &= expect_int("refused: attachment 1 keeps its grey", chan_diff(got1, grey) <= 2, 1);

	glReadBuffer(GL_COLOR_ATTACHMENT0);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(2, tex);
	if (!check_gl_error("drawbuf0 teardown"))
		return -1;
	return ok ? 0 : -1;
}

/* ---- case 128: 108 D in a fragment stage that reads a varying -------------
 *
 * Every program that reads element 0 on the i.MX8MP -- 108 A-D, 116, 117,
 * 120-122 -- has a fragment stage with NO varying input (PS_INPUT_COUNT 1,
 * position only). gl_test 100 idxtex, which passes, reads one (v_col); its
 * constant table, clamp, movar and u5[a.x] read are D's. This is D with
 * idxtex's shape: red and green from the passthrough varying (1, 1), blue
 * and alpha from fcols[fidx], fidx 2 -> (0.6, 1.0). Want (255, 255, 0x99,
 * 255); element 0 reads (255, 255, 0, 255).
 *   PASS -> a fragment stage with no varying input is the defect class
 *   FAIL -> not that either; the knob image (NO_CLAMP + OWN_BASE) decides
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_uniidx_varying(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const float fcols[4][4] = {
		{ 0.0f, 0.0f, 0.0f, 1.0f }, { 1.0f, 0.0f, 0.0f, 1.0f },
		{ 0.2f, 0.4f, 0.6f, 1.0f }, { 0.0f, 1.0f, 0.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint loc;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniidx_j_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	loc = glGetUniformLocation(prog, "fcols");
	if (loc >= 0)
		glUniform4fv(loc, 4, &fcols[0][0]);
	loc = glGetUniformLocation(prog, "fidx");
	if (loc >= 0)
		glUniform1i(loc, 2);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("uniidxj"))
		return -1;
	read_topdown();
	return expect_px("uniidxj: fcols[fidx].ba beside a varying's rg", TW / 2, TH / 2,
			 rgba_u32(255, 255, 0x99, 255), 2) ? 0 : -1;
}

/* ---- case 113: where the texture unit reads each face of each cube level --
 *
 * Every remaining cube failure on the i.MX8MP (texture_functions' cube
 * three, mipmap.cube 28, filtering.cube, specification.*cube*) reads a face
 * other than +X at a level above 0; case 101 (one colour per level on all
 * six faces) and a one-level cube pass. The GL core stores face k of level
 * i at level_off[i] + k * (chain stride * (h >> i)); the texture unit may
 * step faces otherwise at i >= 1. This case asks it: a 16x16 cube, 5 levels,
 * every texel (R, G, B) = (face << 4 | level, x * 16, y * 16), NEAREST, and
 * textureCubeLod aimed at texel (3n/4, n/4) of each face f at each level l
 * (n = 16 >> l; the direction from ES 2.0 Table 3.11). Each fetch is
 * decoded into the log as the (face, level, x, y) it read -- an unwritten
 * address reads 0, which decodes as face 0 level 0 at (0, 0) with R = 0 --
 * and the one recorded check is a 30-bit mask of the (f, l) that fetched
 * the wrong place, bit f * 5 + l.
 * ------------------------------------------------------------------------- */
static int case_cubemap(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static uint32_t img[16 * 16];
	GLuint tex = 0, prog;
	GLint dir_loc, lod_loc;
	uint32_t bad = 0, fits = 0, obs[6][5];
	bool ok = true;

	glGenTextures(1, &tex);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	for (int l = 0; l < 5; l++) {
		int n = 16 >> l;

		for (int f = 0; f < 6; f++) {
			for (int y = 0; y < n; y++)
				for (int x = 0; x < n; x++)
					img[y * n + x] = rgba_u32((uint8_t)(f << 4 | l), (uint8_t)(x * 16),
								  (uint8_t)(y * 16), 255);
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, l, GL_RGBA, n, n, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, img);
		}
	}
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("cubemap setup"))
		return -1;
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);
	prog = link_program(gl_test_cubemap_vs_src, gl_test_cubelod_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_s"), 0);
	dir_loc = glGetUniformLocation(prog, "u_dir");
	lod_loc = glGetUniformLocation(prog, "u_lod");
	for (int f = 0; f < 6; f++) {
		for (int l = 0; l < 5; l++) {
			int n = 16 >> l, x = (3 * n) / 4, y = n / 4;
			float sc = 2.0f * ((float)x + 0.5f) / (float)n - 1.0f;
			float tc = 2.0f * ((float)y + 0.5f) / (float)n - 1.0f;
			float d[3];
			uint32_t px;
			int gf, gl, gx, gy;

			/* ES 2.0 Table 3.11, inverted: the direction whose face and
			 * (sc, tc) are these, with |ma| = 1 */
			switch (f) {
			case 0: d[0] = 1.0f; d[1] = -tc; d[2] = -sc; break;   /* +X: sc -rz, tc -ry */
			case 1: d[0] = -1.0f; d[1] = -tc; d[2] = sc; break;   /* -X: sc rz, tc -ry */
			case 2: d[0] = sc; d[1] = 1.0f; d[2] = tc; break;     /* +Y: sc rx, tc rz */
			case 3: d[0] = sc; d[1] = -1.0f; d[2] = -tc; break;   /* -Y: sc rx, tc -rz */
			case 4: d[0] = sc; d[1] = -tc; d[2] = 1.0f; break;    /* +Z: sc rx, tc -ry */
			default: d[0] = -sc; d[1] = -tc; d[2] = -1.0f; break; /* -Z: sc -rx, tc -ry */
			}
			glUniform3f(dir_loc, d[0], d[1], d[2]);
			glUniform1f(lod_loc, (float)l);
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
			if (!check_gl_error("cubemap draw"))
				return -1;
			read_topdown();
			px = s_px[(TH / 2) * TW + TW / 2];
			obs[f][l] = px;
			gf = (int)(px & 0xff) >> 4;
			gl = (int)(px & 0xff) & 15;
			gx = ((int)((px >> 8) & 0xff) + 8) / 16;
			gy = ((int)((px >> 16) & 0xff) + 8) / 16;
			pr_info("gl_test:   cubemap: face %d level %d texel (%2d,%2d) read face %d level %d (%2d,%2d) 0x%08x %s\n",
				f, l, x, y, gf, gl, gx, gy, px,
				gf == f && gl == l && gx == x && gy == y ? "ok" : "WRONG");
			if (gf != f || gl != l || gx != x || gy != y)
				bad |= 1u << (f * 5 + l);
		}
	}
	/* Which layout does the texture unit read? For each candidate, the byte
	 * address it would fetch for every (face, level, texel) above, mapped back
	 * through the storage the GL core actually wrote -- 64-byte rows (the
	 * chain pitch of a 16-wide level 0) at every level, level l at
	 * 0/6144/9216/10752/11520, six faces of h_l rows each -- to the
	 * fingerprint stored there (0 for padding). A candidate "fits" when all
	 * 30 fetches read what it predicts. Recorded first: bit n = candidate n.
	 *   0 ours: face step 64*h_l, rows 64 (the GL core's own layout)
	 *   1 face step align64(4*w*h), rows 64
	 *   2 face step 4*w*h, rows 4*w (tight)
	 *   3 face step align64(4*w*h), rows 4*w
	 *   4 face step 1024 (level 0's SLICE) at every level, rows 64
	 *   5 face step 1024 >> 2l, rows 64
	 * The i.MX8MP at 2163f271 read faces 1-5 wrong at levels 1-3 only (mask
	 * 0x1ce739c0), and only candidate 1 predicts exactly that mask (host
	 * simulation of this arithmetic): 0 predicts 0, 2 0x3def7bc6, 3
	 * 0x1ce739c6, 4 and 5 0x3def7bc0. */
	{
		static const uint32_t off[5] = { 0, 6144, 9216, 10752, 11520 };

		for (int h = 0; h < 6; h++) {
			bool all = true;

			for (int f = 0; f < 6; f++) {
				for (int l = 0; l < 5; l++) {
					uint32_t n = 16u >> l, x = (3 * n) / 4, y = n / 4, tight = 4 * n * n;
					uint32_t step = h == 0 ? 64 * n : h == 1 || h == 3 ? (tight + 63) & ~63u :
							h == 2 ? tight : h == 4 ? 1024 : 1024u >> (2 * l);
					uint32_t row = h == 2 || h == 3 ? 4 * n : 64;
					uint32_t a = off[l] + (uint32_t)f * step + y * row + x * 4, want = 0;

					for (int L = 4; L >= 0; L--) {
						uint32_t m = 16u >> L, rel;

						if (a < off[L])
							continue;
						rel = a - off[L];
						if (rel < 6 * m * 64) {
							uint32_t F = rel / (m * 64), Y = (rel % (m * 64)) / 64,
								 X = (rel % 64) / 4;

							if (X < m)
								want = rgba_u32((uint8_t)(F << 4 | (uint32_t)L),
										(uint8_t)(X * 16), (uint8_t)(Y * 16), 255);
						}
						break;
					}
					if (chan_diff(obs[f][l], want) > 1)
						all = false;
				}
			}
			pr_info("gl_test:   cubemap: candidate %d %s every fetch\n", h, all ? "fits" : "does not fit");
			if (all)
				fits |= 1u << h;
		}
	}
	/* The wrong-fetch mask first: a board record keeps only the first
	 * failing check, and with the GC7000's tiled copy (0634aced) the fit
	 * mask alone read 0 at 652000b4 -- some fetch wrong, none named. */
	ok &= expect_int("cubemap wrong fetches (bit face*5+level)", (long)bad, 0);
	ok &= expect_int("cubemap layouts that fit every fetch (bit 0 ours .. 5)", (long)fits, 1);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 112: a compare that is both stored and an if's condition --------
 *
 * es100/affectand.frag is dEQP-GLES2 operator.sequence.side_effects.affect_and
 * verbatim: `bool b = (funcA(), u_false == sideEffectA && funcB());` and
 * green only when funcB did not run. The compare is stored to the `&&`'s
 * temporary and is also the `if`'s condition; selection counted the store as
 * its only use, wrote the compare into the slot's register, and the
 * if-converted arm's select read the compare's own register, which nothing
 * wrote. On the host that reads 0 and the picture is right; on the i.MX8MP
 * funcB's store took effect and the case was red (2026-09-24). Drawn for
 * u_false false (green) and true (red: b is true), with a blue clear, so
 * "drew nothing" is its own answer.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_affectand(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint u;
	bool ok = true;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	prog = link_program(gl_test_passthrough_vs_src, gl_test_affectand_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	u = glGetUniformLocation(prog, "u_false");
	if (u < 0) {
		pr_err("gl_test: affectand: u_false not found\n");
		return -1;
	}
	for (int t = 0; t < 2; t++) {
		glUniform1i(u, t);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("affectand"))
			return -1;
		read_topdown();
		ok &= expect_px(t ? "u_false true: red" : "u_false false: green (funcB did not run)", TW / 2, TH / 2,
				t ? 0xff0000ff : 0xff00ff00, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 75: a literal subtracted, in both stages -------------------------
 *
 *     immneg.vert:  gl_Position = vec4(a_pos.xy * 2.0 - 1.0, 0.0, 1.0);
 *     immneg.frag:  gl_FragColor = vec4(u_k.x - 0.5, u_k.y - 1.0, 1.0 - u_k.z, 1.0);
 *
 * The in-process compiler lowered a negated literal by toggling the
 * immediate's NEG field -- payload bit 17 -- so -1.0 was encoded as 5.4e-20
 * and -0.5 as 2.7e-20 (the reference keeps the sign in payload bit 19). Here
 * the quad is given in [0, 1] and must cover the whole target; with the old
 * encoding it covered [0, 2], one quadrant. With u_k = (1.0, 1.25, 0.25) the
 * colour is (0.5, 0.25, 0.75); with the old encoding red and green saturate.
 * Read at the centre and at two opposite corners, with a blue clear.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_immneg(void)
{
	static const float pos[6][2] = {
		{ 0.0f, 0.0f }, { 1.0f, 0.0f }, { 0.0f, 1.0f },
		{ 1.0f, 0.0f }, { 1.0f, 1.0f }, { 0.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint k;
	bool ok = true;
	/* 0.5 -> 128 (127.5 rounds either way), 0.25 -> 64, 0.75 -> 191 */
	const uint32_t want = 0xffbf4080;

	prog = link_program(gl_test_immneg_vs_src, gl_test_immneg_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	k = glGetUniformLocation(prog, "u_k");
	if (k < 0) {
		pr_err("gl_test: immneg u_k not found\n");
		return -1;
	}
	glUniform4f(k, 1.0f, 1.25f, 0.25f, 0.0f);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("immneg"))
		return -1;
	read_topdown();
	ok &= expect_px("immneg: the centre", TW / 2, TH / 2, want, 2);
	ok &= expect_px("immneg: one corner", 1, 1, want, 2);
	ok &= expect_px("immneg: the opposite corner", TW - 2, TH - 2, want, 2);
	return ok ? 0 : -1;
}

/* ---- case 76: bool uniforms, any() and all() ------------------------------
 *
 *     r = (u_b ? 0.5 : 0.0) + (any(u_bv) ? 0.25 : 0.0) + (all(u_bv) ? 0.25 : 0.0)
 *     g = any(greaterThan(u_v, vec4(0.5)))
 *     b = all(greaterThan(u_v, vec4(0.5)))
 *
 * Two things meet here that no earlier case draws. A bool uniform reaches
 * the shader as the GL core stored it -- the integer 1 for true -- while the
 * IR's own true is all ones, so a compiler that tests "== true" instead of
 * "!= 0" reads every set bool uniform as false. And any()/all() reduce
 * across lanes: sides 2 and 4 put the deciding lane at w or leave w alone,
 * so a reduction that reads x, or all four lanes of a bvec3, gets the wrong
 * side.
 *
 * The colours are the IR evaluator's, measured with the same uniform
 * values (lower.rs gl_test_boolany_draws_the_predicted_colours, which holds
 * the table below as BOOLANY_SIDES); they are a prediction for the board,
 * not a board result. Blue clear, read at the centre and a corner.
 *
 * Online-path only, like 68. Host-checked 2026-09-23, merged with main at
 * 63b50b3: both code generators build this shader (GC7000 108 words, USC
 * 154 dwords); it was refused on the USC ("tst/movc/bitwise, not
 * transcribed yet") until usc-backend's select templates landed.
 * ------------------------------------------------------------------------- */
static int case_boolany(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const struct {
		int b, bv[3];
		float v[4];
		uint32_t want;
		const char *what;
	} side[4] = {
		/* 0.25 is 63.75, so 64 */
		{ 1, { 1, 1, 1 }, { 1.0f, 1.0f, 1.0f, 1.0f }, 0xffffffff,
		  "all true: white" },
		{ 0, { 0, 0, 1 }, { 0.0f, 0.0f, 0.0f, 1.0f }, 0xff00ff40,
		  "u_bv.z and u_v.w alone: any true, all false" },
		{ 1, { 0, 0, 0 }, { 0.0f, 0.0f, 0.0f, 0.0f }, 0xff000080,
		  "u_b alone: red 0.5, nothing else" },
		{ 0, { 1, 1, 0 }, { 1.0f, 1.0f, 1.0f, 0.0f }, 0xff00ff40,
		  "all but the last lane: any true, all false" },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint b, bv, uv;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_boolany_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	b = glGetUniformLocation(prog, "u_b");
	bv = glGetUniformLocation(prog, "u_bv");
	uv = glGetUniformLocation(prog, "u_v");
	if (b < 0 || bv < 0 || uv < 0) {
		pr_err("gl_test: boolany uniforms not found (%d %d %d)\n", b, bv, uv);
		return -1;
	}
	for (int s = 0; s < 4; s++) {
		glUniform1i(b, side[s].b);
		glUniform3i(bv, side[s].bv[0], side[s].bv[1], side[s].bv[2]);
		glUniform4f(uv, side[s].v[0], side[s].v[1], side[s].v[2], side[s].v[3]);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(side[s].what))
			return -1;
		read_topdown();
		ok &= expect_px(side[s].what, TW / 2, TH / 2, side[s].want, 2);
		ok &= expect_px(side[s].what, 1, 1, side[s].want, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 77: the transcendentals, scalar and on vectors ------------------
 *
 *     transc.frag:   vec4(sin(pi/6), cos(pi/3), log2(2.0) / 4, exp2(-1.0))
 *                    = (0.5, 0.5, 0.25, 0.5)
 *     vtransc.frag:  vec4(sin(vec3(pi/6, pi/2, 0.0)), inversesqrt(vec2(1, 4)).y)
 *                    = (0.5, 1.0, 0.0, 0.5)
 *
 * This core's transcendental unit takes sin/cos arguments in units of pi and
 * returns sin, cos and log as a pair of lanes to be multiplied (Mesa's
 * etna_lower_alu; the reference's `sin.rtz t.zw` + `mul t.z, t.w`). The
 * in-process compiler emitted a plain `sin` of radians and used one lane of
 * the pair; it also emitted a scalar operation once per vector, so every lane
 * got the first lane's result. Blending is off, so alpha is stored as is.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_transc(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	const float pi = 3.14159265f;
	struct pc_vertex v[6];
	bool ok = true;

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	for (int p = 0; p < 2; p++) {
		GLuint prog = link_program(gl_test_passthrough_vs_src, p ? gl_test_vtransc_fs_src : gl_test_transc_fs_src);
		GLint u;

		if (!prog)
			return -1;
		glUseProgram(prog);
		upload_vbo(v, sizeof(v));
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
		glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		u = glGetUniformLocation(prog, p ? "u_v" : "u_a");
		if (u < 0) {
			pr_err("gl_test: transc: uniform not found (program %d)\n", p);
			return -1;
		}
		if (p)
			glUniform4f(u, pi / 6.0f, pi / 2.0f, 0.0f, 4.0f);
		else
			glUniform4f(u, pi / 6.0f, pi / 3.0f, 2.0f, -1.0f);
		glDisable(GL_BLEND);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(p ? "vtransc" : "transc"))
			return -1;
		read_topdown();
		ok &= expect_px(p ? "vtransc: sin(vec3), inversesqrt(vec2).y" : "transc: sin, cos, log2, exp2",
				TW / 2, TH / 2, p ? 0x8000ff80 : 0x80408080, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 78: a matrix varying --------------------------------------------
 *
 *     matvary.vert:  v_m = mat2(a_color); v_after = vec4(0.5);
 *     matvary.frag:  gl_FragColor = vec4(v_m[0].x, v_m[1].y, v_after.z, 1.0)
 *
 * A mat2 varying is one declaration over two locations. The in-process
 * compiler numbered varyings by declaration and refused it; now it takes a
 * register per column, its io rows are named v_m and v_m[1] so the link pairs
 * them, and v_after moves up a register. With a_color = (0.25, 0, 0, 0.75):
 * (0.25, 0.75, 0.5, 1.0). A column in the wrong register, or v_after in the
 * matrix's second, gives another colour.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_matvary(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint lp, lc;
	bool ok = true;

	prog = link_program(gl_test_matvary_vs_src, gl_test_matvary_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = 0.25f; v[i].color[1] = 0.0f; v[i].color[2] = 0.0f; v[i].color[3] = 0.75f;
	}
	upload_vbo(v, sizeof(v));
	lp = glGetAttribLocation(prog, "a_pos");
	lc = glGetAttribLocation(prog, "a_color");
	if (lp < 0 || lc < 0) {
		pr_err("gl_test: matvary attribute locations a_pos %d a_color %d\n", lp, lc);
		return -1;
	}
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer((GLuint)lc, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lc);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("matvary"))
		return -1;
	read_topdown();
	ok &= expect_px("matvary: both columns and the varying after", TW / 2, TH / 2, 0xff80bf40, 2);
	return ok ? 0 : -1;
}

/* ---- case 79: integer division ---------------------------------------------
 *
 *     idiv.frag, with u_d = (0.7, -0.7, 1.0, 1.0):
 *       ivec2(7, -7) / ivec2(2, 3) = (3, -2);  100 / (7 + 1) = 12
 *       gl_FragColor = (3/4, (-2+4)/4, 12/16, 1) = (0.75, 0.5, 0.75, 1.0)
 *
 * The in-process compiler refused integer division. It is now lowered per
 * lane in float with an integer remainder correction (no idiv0: neither the
 * reference nor Mesa uses it). A quotient rounded instead of truncated, or a
 * lost sign, gives another colour.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_idiv(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint u;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_idiv_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	u = glGetUniformLocation(prog, "u_d");
	if (u < 0) {
		pr_err("gl_test: idiv u_d not found\n");
		return -1;
	}
	glUniform4f(u, 0.7f, -0.7f, 1.0f, 1.0f);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("idiv"))
		return -1;
	read_topdown();
	ok &= expect_px("idiv: (3, -2) and 12", TW / 2, TH / 2, 0xffbf80bf, 2);
	return ok ? 0 : -1;
}

/* ---- case 80: matrix arithmetic, a vector ?:, dynamic indexing -------------
 *
 *     m = mat2(u_k) = columns (1, 2) and (0.5, 4) -- NOT symmetric, so a
 *     transposed product or a column read as a row moves a channel
 *     red   = (s[1][1] + d[1][0] + q[0][1] + n[1][0] + 4 p[1][0]) / 16
 *             s = m+m, d = 4-m, q = 2/m, n = -m, p = m++  -> 0.875
 *     green = ((m*=m)[1][0] - m[0][0]) / 8 + t.x + t.y,
 *             t = u_c > 0.5 ? vec2(0.5, 0.0) : vec2(0.0, 0.25)
 *             -> 0.75 on the true side, 0.5 on the false side
 *     blue  = arr[i] + p[i][i-1] / 4 + v[j] + arr[2], i = u_i = 1, j = i+1,
 *             after arr[j] = arr[i] * 0.5            -> 0.75
 *
 * The colours are the IR evaluator's with these uniforms (lower.rs
 * gl_test_matdyn_draws_the_predicted_colours) and agree with the working
 * above; a prediction for the board, not a board result. Both sides of the
 * condition are drawn, for the reason case 68 gives. Blue clear.
 *
 * Online-path only, like 68. Host-checked 2026-09-23, merged with main at
 * 63b50b3: both code generators build this shader (GC7000 372 words, USC
 * 248 dwords), so the AM62P can run it too.
 * ------------------------------------------------------------------------- */
static int case_matdyn(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	/* 0.875 -> 223, 0.75 -> 191, 0.5 -> 128 (127.5 rounds either way) */
	static const struct { float c; uint32_t want; const char *what; } side[2] = {
		{ 1.0f, 0xffbfbfdf, "matdyn, u_c true: green 0.75" },
		{ 0.0f, 0xffbf80df, "matdyn, u_c false: green 0.5" },
	};
	struct pc_vertex v[6];
	GLuint prog;
	GLint k, c, i;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_matdyn_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int n = 0; n < 6; n++) {
		v[n].pos[0] = pos[n][0]; v[n].pos[1] = pos[n][1];
		v[n].pos[2] = 0.0f; v[n].pos[3] = 1.0f;
		v[n].color[0] = v[n].color[1] = v[n].color[2] = v[n].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	k = glGetUniformLocation(prog, "u_k");
	c = glGetUniformLocation(prog, "u_c");
	i = glGetUniformLocation(prog, "u_i");
	if (k < 0 || c < 0 || i < 0) {
		pr_err("gl_test: matdyn uniforms not found (%d %d %d)\n", k, c, i);
		return -1;
	}
	glUniform4f(k, 1.0f, 2.0f, 0.5f, 4.0f);
	glUniform1i(i, 1);
	for (int s = 0; s < 2; s++) {
		glUniform1f(c, side[s].c);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(side[s].what))
			return -1;
		read_topdown();
		ok &= expect_px(side[s].what, TW / 2, TH / 2, side[s].want, 2);
		ok &= expect_px(side[s].what, 1, 1, side[s].want, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 90: a thousand programs made and deleted -------------------------
 *
 * The in-process compiler keeps each compiled shader in a bounded region
 * (512 KiB, store.rs KEEP_BYTES) because the GL core's copy points into it.
 * Until d13862d nothing gave it back: create/compile/link/delete walked up to
 * the ceiling in about 192 programs and then every compile was refused with
 * "out of compiler memory". On Zephyr and bare metal that region is the only
 * memory the compiler has, so this is the product defect and the board is
 * where the fix has to be shown.
 *
 * Each cycle makes a vertex and a fragment shader -- the fragment text differs
 * by a literal (`* 0.0`, so the colour does not), which defeats the compile
 * cache the way distinct application shaders do -- links them, draws a full
 * quad, and deletes the program and both shaders. The region is read through
 * shc_online_keep_stats() before the first cycle, after 200 and after the
 * last. PASS: no cycle refused, the last draw's colour at two pixels, the
 * bytes held at the end equal to those held at the start, and the high-water
 * after 200 cycles equal to the one at the end (it may rise over the first
 * few while the region settles; it must not climb with the count).
 *
 * The prediction, written before any board run: 0 refusals, 0xffbf8040 at
 * both pixels (0.25, 0.5, 0.75), end == start, peak(200) == peak(1000).
 *
 * Online-path only: a build without the in-process compiler has no
 * shc_online_keep_stats (it is weak here) and the case says so and fails
 * rather than pass on a region it could not see.
 * ------------------------------------------------------------------------- */
extern int shc_online_keep_stats(size_t *in_use, size_t *peak) __attribute__((weak));
/* gl_test's closing line: the shader back end's two counters, each in the
 * image only with its tier (gles_shaderc.h), weak like the keep stats. */
extern uint32_t shc_precompiled_hits(void) __attribute__((weak));
extern uint32_t shc_online_compiler_runs(void) __attribute__((weak));

static int case_churn(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const char vs_src[] =
		"attribute vec4 a_pos;\n"
		"void main() { gl_Position = a_pos; }\n";
	const int cycles = 1000;
	struct pc_vertex v[6];
	GLuint vbo;
	size_t used0 = 0, peak0 = 0, used200 = 0, peak200 = 0, used_end = 0, peak_end = 0;
	int refused = 0;
	bool ok = true;

	if (!shc_online_keep_stats) {
		pr_err("gl_test: churn: this build has no in-process compiler (shc_online_keep_stats)\n");
		return -1;
	}
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	/* One buffer for every cycle: the region measured is the compiler's,
	 * and a thousand buffers would measure the heap instead. */
	vbo = upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glEnableVertexAttribArray(0);
	shc_online_keep_stats(&used0, &peak0);
	pr_info("gl_test: churn: keep before: %u held, high-water %u\n", (unsigned)used0, (unsigned)peak0);

	for (int n = 0; n < cycles; n++) {
		char fs_src[160];
		const char *vp = vs_src, *fp = fs_src;
		GLuint vs, fs, prog;
		GLint cv = 0, cf = 0, lk = 0;

		snprintf(fs_src, sizeof(fs_src),
			 "precision mediump float;\n"
			 "void main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0 + %d.0 * 0.0); }\n", n);
		vs = glCreateShader(GL_VERTEX_SHADER);
		fs = glCreateShader(GL_FRAGMENT_SHADER);
		glShaderSource(vs, 1, &vp, NULL);
		glShaderSource(fs, 1, &fp, NULL);
		glCompileShader(vs);
		glCompileShader(fs);
		glGetShaderiv(vs, GL_COMPILE_STATUS, &cv);
		glGetShaderiv(fs, GL_COMPILE_STATUS, &cf);
		prog = glCreateProgram();
		glAttachShader(prog, vs);
		glAttachShader(prog, fs);
		glBindAttribLocation(prog, 0, "a_pos");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &lk);
		if (!cv || !cf || !lk) {
			char log[256] = "";

			glGetShaderInfoLog(cv ? fs : vs, sizeof(log), NULL, log);
			if (refused < 3)
				pr_err("gl_test: churn: cycle %d refused (vs %d fs %d link %d): %s\n", n, cv, cf, lk, log);
			refused++;
		} else {
			glUseProgram(prog);
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			/* drain now and then, so the draws do not pile up behind the
			 * compiles; the last one is read back below */
			if (n % 50 == 49 || n == cycles - 1)
				glFinish();
		}
		if (n == cycles - 1 && lk) {
			if (!check_gl_error("churn, the last draw"))
				ok = false;
			read_topdown();
			ok &= expect_px("churn: the last program's colour, centre", TW / 2, TH / 2, 0xffbf8040, 2);
			ok &= expect_px("churn: and a corner", 1, 1, 0xffbf8040, 2);
		}
		glUseProgram(0);
		glDeleteProgram(prog);
		glDeleteShader(vs);
		glDeleteShader(fs);
		if (n == 199)
			shc_online_keep_stats(&used200, &peak200);
	}
	glDeleteBuffers(1, &vbo);
	shc_online_keep_stats(&used_end, &peak_end);
	pr_info("gl_test: churn: %d cycles, %d refused; keep held %u -> %u (after 200: %u), high-water %u -> %u (after 200: %u)\n",
		cycles, refused, (unsigned)used0, (unsigned)used_end, (unsigned)used200,
		(unsigned)peak0, (unsigned)peak_end, (unsigned)peak200);
	if (refused) {
		pr_err("gl_test: churn: %d of %d cycles were refused\n", refused, cycles);
		ok = false;
	}
	if (used_end != used0) {
		pr_err("gl_test: churn: the region holds %u bytes more than before\n", (unsigned)(used_end - used0));
		ok = false;
	}
	if (peak_end != peak200) {
		pr_err("gl_test: churn: the high-water climbed after 200 programs (%u -> %u)\n",
		       (unsigned)peak200, (unsigned)peak_end);
		ok = false;
	}
	return ok ? 0 : -1;
}

/* ---- case 81: branches whose condition differs from pixel to pixel --------
 *
 *     if (v_color.y > 0.5) discard;   ...   if (v_color.w > 0.5) discard;
 *     gl_FragColor = green;
 *
 * Case 73's redesign. Its conditions came from a uniform, so every pixel took
 * the same branch, and on silicon it passed with or without the fix it was
 * written for. Here v_color = (1 - t, t, 1 - s, s), s the horizontal and t
 * the vertical position (0 at the bottom-left corner, 1 at the top-right), so
 * the y test discards the top half and the w test the right half: the
 * bottom-left quadrant is green and the other three keep the blue clear. A
 * test of lane x instead of y discards the bottom half, one of z instead of
 * w the left half, so any branch that reads the wrong lane moves the green
 * quadrant; the quadrants' centres are read, well away from the 0.5 edges.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_branchquad(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	/* read top-down: row 0 is the top of the quad */
	static const struct { int x, y; uint32_t want; const char *what; } quad[4] = {
		{ TW / 4,     3 * TH / 4, 0xff00ff00, "branchquad: bottom-left, neither test true: green" },
		{ 3 * TW / 4, 3 * TH / 4, 0xffff0000, "branchquad: bottom-right, the w test true: the clear" },
		{ TW / 4,     TH / 4,     0xffff0000, "branchquad: top-left, the y test true: the clear" },
		{ 3 * TW / 4, TH / 4,     0xffff0000, "branchquad: top-right, both tests true: the clear" },
	};
	struct pc_vertex v[6];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_branchquad_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		float s = (pos[i][0] + 1.0f) * 0.5f, t = (pos[i][1] + 1.0f) * 0.5f;

		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = 1.0f - t; v[i].color[1] = t;
		v[i].color[2] = 1.0f - s; v[i].color[3] = s;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("branchquad"))
		return -1;
	read_topdown();
	for (int q = 0; q < 4; q++)
		ok &= expect_px(quad[q].what, quad[q].x, quad[q].y, quad[q].want, 2);
	return ok ? 0 : -1;
}

/* ---- case 86: any()/all() over vectors narrower than four -----------------
 *
 *     e = all(equal(a_uv, vec2(0.0)))        a vec2 attribute
 *     f = any(greaterThan(a_n, vec3(0.5)))   a vec3 attribute
 *     v_color = (e, f, 0, 1)
 *
 * The reductions fold all four lanes, so a bvec2 or bvec3 must have a real
 * lane repeated into the spare ones. Folded as they came, they read the
 * vertex fetch's padding -- (z, w) = (0, 1) for a vec2, w = 1 for a vec3 --
 * and all() of a vec2 that IS zero came out false (Qt's smoothtexture.vert
 * outerEdge), any() of a vec3 below 0.5 came out true. Two draws, a blue
 * clear before each, every vertex of a draw alike:
 *     a_uv = (0, 0),   a_n = (0, 0, 0.9)      -> (1, 1, 0, 1)  yellow
 *     a_uv = (0, 0.5), a_n = (0.1, 0.2, 0.3)  -> (0, 0, 0, 1)  black
 * Folding the padding instead gives green (0, 1, 0, 1) for both: the vec2's
 * w = 1 is not 0, and the vec3's w = 1 is above 0.5.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_anyall(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const struct { float uv[2], n[3]; uint32_t want; const char *what; } draw[2] = {
		{ { 0.0f, 0.0f }, { 0.0f, 0.0f, 0.9f }, 0xff00ffff, "anyall: all(uv == 0) true, any(n > 0.5) true: yellow" },
		{ { 0.0f, 0.5f }, { 0.1f, 0.2f, 0.3f }, 0xff000000, "anyall: all(uv == 0) false, any(n > 0.5) false: black" },
	};
	struct { float pos[4], uv[2], n[3]; } v[6];
	GLuint prog;
	GLint lp, lu, ln;
	bool ok = true;

	prog = link_program(gl_test_anyall_vs_src, gl_test_interp_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	lp = glGetAttribLocation(prog, "a_pos");
	lu = glGetAttribLocation(prog, "a_uv");
	ln = glGetAttribLocation(prog, "a_n");
	if (lp < 0 || lu < 0 || ln < 0) {
		pr_err("gl_test: anyall attributes a_pos %d a_uv %d a_n %d\n", lp, lu, ln);
		return -1;
	}
	for (int d = 0; d < 2; d++) {
		for (int i = 0; i < 6; i++) {
			v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
			v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
			v[i].uv[0] = draw[d].uv[0]; v[i].uv[1] = draw[d].uv[1];
			v[i].n[0] = draw[d].n[0]; v[i].n[1] = draw[d].n[1]; v[i].n[2] = draw[d].n[2];
		}
		upload_vbo(v, sizeof(v));
		glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(v[0]), (const void *)0);
		glVertexAttribPointer((GLuint)lu, 2, GL_FLOAT, GL_FALSE, sizeof(v[0]), (const void *)16);
		glVertexAttribPointer((GLuint)ln, 3, GL_FLOAT, GL_FALSE, sizeof(v[0]), (const void *)24);
		glEnableVertexAttribArray((GLuint)lp);
		glEnableVertexAttribArray((GLuint)lu);
		glEnableVertexAttribArray((GLuint)ln);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(draw[d].what))
			return -1;
		read_topdown();
		ok &= expect_px(draw[d].what, TW / 2, TH / 2, draw[d].want, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 84: gl_FragCoord on the default framebuffer and on an FBO -------
 *
 *     gl_FragColor = (gl_FragCoord.x / 64, gl_FragCoord.y / 64, 0, 1)
 *
 * gl_FragCoord is window-relative with its origin at the BOTTOM left (ES 1.00
 * 7.2). The default framebuffer is stored top-down here and drawn with the
 * viewport mirrored (flip_y); a framebuffer object is not. The rasteriser
 * hands the fragment stage the position in the TARGET's rows, so unless the
 * shader is told to flip, the default framebuffer's gl_FragCoord.y counts
 * from the top. Read top-down, row r is window y = 63.5 - r:
 *     row 4  -> green 59.5 / 64 = 237     row 59 -> green 4.5 / 64 = 18
 * and column 8 is red 8.5 / 64 = 34, on both targets. A y counted from the
 * top reads 18 at row 4 and 237 at row 59.
 * ------------------------------------------------------------------------- */
static int case_fragcoord(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static const struct { int x, y; uint32_t want; const char *what; } px[2] = {
		{ 8, 4,  0xff00ed22, "row 4 (window y 59.5): green 237, red 34" },
		{ 8, 59, 0xff001222, "row 59 (window y 4.5): green 18, red 34" },
	};
	struct pc_vertex v[6];
	GLuint prog, tex = 0, fbo = 0;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_fragcoord_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	for (int target = 0; target < 2; target++) {
		if (target == 1) {
			glGenTextures(1, &tex);
			glBindTexture(GL_TEXTURE_2D, tex);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
			glGenFramebuffers(1, &fbo);
			glBindFramebuffer(GL_FRAMEBUFFER, fbo);
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
		}
		glViewport(0, 0, TW, TH);
		glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(target ? "fragcoord: fbo" : "fragcoord: default framebuffer"))
			return -1;
		read_topdown();
		for (int p = 0; p < 2; p++) {
			char what[96];

			snprintf(what, sizeof(what), "fragcoord: %s, %s", target ? "fbo" : "default framebuffer", px[p].what);
			ok &= expect_px(what, px[p].x, px[p].y, px[p].want, 3);
		}
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 162: depth kept across framebuffers (dskeep) ---------------------
 *
 * A backend keeps DSK_SLOTS depth/stencil buffers (src/ds_retention.rs
 * DS_SLOTS, the same on every GPU). They
 * hold GL-visible contents: a renderbuffer's image persists across
 * framebuffer switches (ES 2.0 4.4.3). One more used to give up the oldest
 * with no error, and the first framebuffer's depth was gone. Now the draw
 * that needs one more is GL_OUT_OF_MEMORY, and a buffer is free again only
 * when its renderbuffer is deleted (or its contents are one clear value).
 *
 * Framebuffers 0, 1, ... each get a colour texture and a DEPTH_COMPONENT16
 * renderbuffer, are cleared (black, depth 1) and take a green quad at z 0
 * (depth 0.5) with the depth test LESS, until a draw is GL_OUT_OF_MEMORY.
 * The window's own buffer, if an earlier case drew with depth, is one of
 * the DSK_SLOTS, so the refusal comes at framebuffer DSK_SLOTS or before;
 * one that never comes within DSK_MAX is a failure (the old eviction). Then
 * framebuffer 0
 * takes a blue quad from z -1 at the bottom left to +1 at the top right:
 * with its depth kept it passes below the diagonal only (the green stays
 * above); with the depth lost (1.0) it would be blue everywhere. Last,
 * framebuffer 0's depth renderbuffer is deleted while it is bound (so no
 * framebuffer holds it and it is freed), and the refused framebuffer's
 * clear and quad draw, read back green.
 * ------------------------------------------------------------------------- */
#define DSK_SLOTS 32 /* src/ds_retention.rs DS_SLOTS */
#define DSK_MAX (DSK_SLOTS + 1)



#define DSK_GREEN 0xff00ff00u
#define DSK_BLUE  0xffff0000u

/* A quad from (-1, -1, z00) to (1, 1, z11), the other corners at the mean. */
static void dsk_quad(GLuint prog, uint32_t rgba, float z00, float z11)
{
	const float m = 0.5f * (z00 + z11);
	const float v[18] = { -1, -1, z00, 1, -1, m, -1, 1, m, -1, 1, m, 1, -1, m, 1, 1, z11 };
	GLint lp = glGetAttribLocation(prog, "a_pos");

	glUseProgram(prog);
	glUniform4f(glGetUniformLocation(prog, "u_color"), (float)(rgba & 0xff) / 255.0f,
		    (float)((rgba >> 8) & 0xff) / 255.0f, (float)((rgba >> 16) & 0xff) / 255.0f,
		    (float)(rgba >> 24) / 255.0f);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 3, GL_FLOAT, GL_FALSE, 12, v);
	glEnableVertexAttribArray((GLuint)lp);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray((GLuint)lp);
}

static int case_dskeep(void)
{
	GLuint prog, col[DSK_MAX], depth[DSK_MAX], fbo[DSK_MAX];
	int made = 0, refused = -1;
	uint32_t right = 0, judged = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_dsk_vs_src, gl_test_dsk_fs_src);
	if (!prog)
		return -1;
	glGenFramebuffers(DSK_MAX, fbo);
	glViewport(0, 0, TW, TH);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	for (int i = 0; i < DSK_MAX && refused < 0; i++, made++) {
		GLenum e;

		glGenTextures(1, &col[i]);
		glBindTexture(GL_TEXTURE_2D, col[i]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glBindTexture(GL_TEXTURE_2D, 0);
		glGenRenderbuffers(1, &depth[i]);
		glBindRenderbuffer(GL_RENDERBUFFER, depth[i]);
		glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, TW, TH);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col[i], 0);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth[i]);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: dskeep: framebuffer %d incomplete\n", i);
			ok = false;
			break;
		}
		glClearColor(0, 0, 0, 1);
		glClearDepthf(1.0f);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		dsk_quad(prog, DSK_GREEN, 0.0f, 0.0f);
		e = glGetError();
		if (e == GL_OUT_OF_MEMORY)
			refused = i;
		else if (e != GL_NO_ERROR) {
			pr_err("gl_test: dskeep: GL error 0x%x at framebuffer %d\n", (unsigned)e, i);
			ok = false;
			break;
		}
		glFinish();
	}
	pr_info("gl_test: dskeep: %d framebuffer(s) kept their depth, framebuffer %d refused (GL_OUT_OF_MEMORY)\n",
		refused < 0 ? made : refused, refused);
	/* refused by index DSK_SLOTS at the latest, and not before one
	 * framebuffer kept its depth */
	ok &= gl_test_check(refused >= 1 && refused < DSK_MAX, (uint32_t)refused, DSK_MAX - 1);
	if (ok) {
		/* framebuffer 0's depth, through a depth-tested quad */
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[0]);
		dsk_quad(prog, DSK_BLUE, -1.0f, 1.0f);
		glFinish();
		ok &= check_gl_error("dskeep: framebuffer 0 again");
		read_topdown();
		for (uint32_t ty = 0; ty < TH; ty++)
			for (uint32_t x = 0; x < TW; x++) {
				int d = (int)(x + (TH - 1 - ty) + 1) - (int)TW;

				if (d >= -1 && d <= 1)
					continue; /* the diagonal itself */
				judged++;
				right += chan_diff(s_px[ty * TW + x], d < 0 ? DSK_BLUE : DSK_GREEN) <= 1;
			}
		pr_info("gl_test: dskeep: framebuffer 0: %u of %u pixels as its kept depth draws them\n", right,
			judged);
		ok &= gl_test_check(judged > 0 && right == judged, right, judged);
		ok &= expect_px("dskeep: fb 0 below the diagonal", 8, TH - 9, DSK_BLUE, 1);
		ok &= expect_px("dskeep: fb 0 above the diagonal", TW - 9, 8, DSK_GREEN, 1);

		/* the refused framebuffer draws once framebuffer 0's depth goes */
		glDeleteRenderbuffers(1, &depth[0]);
		depth[0] = 0;
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[refused]);
		glClearColor(0, 0, 0, 1);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		dsk_quad(prog, DSK_GREEN, 0.0f, 0.0f);
		glFinish();
		ok &= check_gl_error("dskeep: the refused framebuffer, after a delete");
		read_topdown();
		ok &= expect_px("dskeep: the refused fb draws", TW / 2, TH / 2, DSK_GREEN, 1);
	}
	glDisable(GL_DEPTH_TEST);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(DSK_MAX, fbo);
	for (int i = 0; i < made; i++) {
		if (depth[i])
			glDeleteRenderbuffers(1, &depth[i]);
		glDeleteTextures(1, &col[i]);
	}
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 167: femtovg's stencil-only layers (stcycle) ---------------------
 *
 * femtovg (the Slint cluster's renderer) makes one framebuffer per layer
 * image: a colour texture and a STENCIL_INDEX8 renderbuffer, nothing on the
 * depth point. It clears colour and stencil with a scissor over the whole
 * image and fills concave paths through the stencil. On the Pi 4 the
 * cluster's frame 1 made 13 of them; with eight depth/stencil buffers the
 * 9th to 13th layers' draws were GL_OUT_OF_MEMORY and the layers were
 * missing (records/pi4/record_DEMOG1_cluster_demog1.txt, crc32 0xc63336bf
 * instead of 0xf59828d6). Every GPU keeps DSK_SLOTS now, so the case makes
 * the cluster's 13 on all of them.
 *
 * Each framebuffer i: clear (black, stencil 0, scissor = the whole image),
 * a stencil pass (colour masked, INCR where drawn) over a triangle, lower
 * left for even i and upper right for odd, and a red cover pass (stencil
 * NOTEQUAL 0, KEEP). The stencil it leaves (1 in its triangle) is GL's to
 * keep. No draw may be GL_OUT_OF_MEMORY. Then each framebuffer again with
 * no clear: a whole-image green quad with stencil EQUAL 1 lands in its own
 * triangle only. A lost stencil leaves that triangle red; a stencil from
 * another framebuffer paints the other half green.
 * ------------------------------------------------------------------------- */
#define STC_LAYERS 13 /* the cluster's frame 1 */
#define STC_RED 0xff0000ffu

/* The triangle of framebuffer i: lower left (even) or upper right (odd). */
static void stc_triangle(GLuint prog, int i, uint32_t rgba)
{
	static const float lower_left[9] = { -1, -1, 0, 1, -1, 0, -1, 1, 0 };
	static const float upper_right[9] = { 1, -1, 0, 1, 1, 0, -1, 1, 0 };
	GLint lp = glGetAttribLocation(prog, "a_pos");

	glUseProgram(prog);
	glUniform4f(glGetUniformLocation(prog, "u_color"), (float)(rgba & 0xff) / 255.0f,
		    (float)((rgba >> 8) & 0xff) / 255.0f, (float)((rgba >> 16) & 0xff) / 255.0f,
		    (float)(rgba >> 24) / 255.0f);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 3, GL_FLOAT, GL_FALSE, 12, (i & 1) ? upper_right : lower_left);
	glEnableVertexAttribArray((GLuint)lp);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glDisableVertexAttribArray((GLuint)lp);
}

static int case_stcycle(void)
{
	GLuint prog, col[STC_LAYERS], st[STC_LAYERS], fbo[STC_LAYERS];
	int made = 0, oom = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_dsk_vs_src, gl_test_dsk_fs_src);
	if (!prog)
		return -1;
	glGenFramebuffers(STC_LAYERS, fbo);
	glViewport(0, 0, TW, TH);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_CULL_FACE);
	glStencilMask(0xff);
	for (int i = 0; i < STC_LAYERS; i++, made++) {
		GLenum e;

		glGenTextures(1, &col[i]);
		glBindTexture(GL_TEXTURE_2D, col[i]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glBindTexture(GL_TEXTURE_2D, 0);
		glGenRenderbuffers(1, &st[i]);
		glBindRenderbuffer(GL_RENDERBUFFER, st[i]);
		glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, TW, TH);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col[i], 0);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, st[i]);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
			pr_err("gl_test: stcycle: framebuffer %d incomplete\n", i);
			ok = false;
			made++;
			break;
		}
		/* femtovg's clear_rect: colour and stencil, a scissor over the
		 * whole image */
		glEnable(GL_SCISSOR_TEST);
		glScissor(0, 0, TW, TH);
		glClearColor(0, 0, 0, 1);
		glClearStencil(0);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		glDisable(GL_SCISSOR_TEST);
		/* the stencil pass, then the cover pass */
		glEnable(GL_STENCIL_TEST);
		glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
		glStencilFunc(GL_ALWAYS, 0, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
		stc_triangle(prog, i, STC_RED);
		glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		glStencilFunc(GL_NOTEQUAL, 0, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		dsk_quad(prog, STC_RED, 0.0f, 0.0f);
		glDisable(GL_STENCIL_TEST);
		e = glGetError();
		if (e == GL_OUT_OF_MEMORY) {
			oom++;
		} else if (e != GL_NO_ERROR) {
			pr_err("gl_test: stcycle: GL error 0x%x at framebuffer %d\n", (unsigned)e, i);
			ok = false;
			made++;
			break;
		}
		glFinish();
	}
	pr_info("gl_test: stcycle: %d framebuffer(s), %d with a draw refused (GL_OUT_OF_MEMORY)\n", made, oom);
	ok &= gl_test_check(oom == 0 && made == STC_LAYERS, (uint32_t)oom, 0);
	/* each framebuffer's stencil, read through a stencil-tested quad */
	for (int i = 0; ok && i < STC_LAYERS; i++) {
		char what[48];

		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glEnable(GL_STENCIL_TEST);
		glStencilFunc(GL_EQUAL, 1, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		dsk_quad(prog, DSK_GREEN, 0.0f, 0.0f);
		glDisable(GL_STENCIL_TEST);
		glFinish();
		ok &= check_gl_error("stcycle: a framebuffer again");
		read_topdown();
		snprintf(what, sizeof(what), "stcycle: fb %d lower left", i);
		ok &= expect_px(what, 8, TH - 9, (i & 1) ? 0xff000000u : DSK_GREEN, 1);
		snprintf(what, sizeof(what), "stcycle: fb %d upper right", i);
		ok &= expect_px(what, TW - 9, 8, (i & 1) ? DSK_GREEN : 0xff000000u, 1);
	}
	/* the stencil state back to GL's defaults for the cases after */
	glStencilFunc(GL_ALWAYS, 0, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(STC_LAYERS, fbo);
	for (int i = 0; i < made; i++) {
		glDeleteRenderbuffers(1, &st[i]);
		glDeleteTextures(1, &col[i]);
	}
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 168: femtovg's layers alive together (stlayers) -----------------
 *
 * Case 167 with the cluster's numbers: 13 layer framebuffers of 600x600,
 * each a colour texture and a STENCIL_INDEX8 renderbuffer only, all 13
 * alive together (femtovg keeps a frame's layer images until the frame is
 * composed). Each layer: the scissored whole clear, a stencil pass (INCR)
 * over its triangle and a red cover pass (stencil NOTEQUAL 0), as in 167.
 * Then, once all 13 exist, every layer again with no clear: a green quad
 * with stencil EQUAL 1, and two pixels read back from the layer itself (its
 * triangle green, the other half black).
 *
 * The number of layers refused (GL_OUT_OF_MEMORY) is a measurement, not a
 * pass condition: a backend keeps DSK_SLOTS depth/stencil buffers, more
 * than 13, so a refusal says GPU memory ran out first (the AM62P's render
 * pool held 3 of these layers in 2026-09-29's run). The
 * case passes when every error is GL_OUT_OF_MEMORY (the backend's named
 * refusal; its line is in the run's log), every layer drawn on the first
 * pass draws again and reads back its own stencil, and no layer refused on
 * the first pass is read as if it had been drawn. A colour texture that
 * could not be allocated is counted apart (the carve-out, not the slots)
 * and ends the layers: the ones made are still drawn again and read back.
 * ------------------------------------------------------------------------- */
#define STL_LAYERS 13 /* the cluster's frame 1 */
#define STL_W 600
#define STL_H 600

/* One pixel of the bound framebuffer, GL window coordinates. */
static bool stl_px(int i, const char *where, int x, int y, uint32_t want)
{
	uint32_t got = 0;
	bool ok;

	glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &got);
	ok = chan_diff(got, want) <= 1;
	pr_info("gl_test:   stlayers: fb %2d %-11s (%3d,%3d) = 0x%08x  want 0x%08x  %s  [check %d]\n", i,
		where, x, y, got, want, ok ? "ok" : "MISMATCH", gl_test_check_no + 1);
	return gl_test_check(ok, got, want);
}

static int case_stlayers(void)
{
	GLuint prog, col[STL_LAYERS] = { 0 }, st[STL_LAYERS] = { 0 }, fbo[STL_LAYERS];
	bool drawn[STL_LAYERS] = { false };
	int made = 0, complete = 0, oom = 0, notex = 0, oom2 = 0, lost = 0, read = 0, unchecked = 0;
	bool ok = true;

	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	prog = link_program(gl_test_dsk_vs_src, gl_test_dsk_fs_src);
	if (!prog)
		return -1;
	glGenFramebuffers(STL_LAYERS, fbo);
	glViewport(0, 0, STL_W, STL_H);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_CULL_FACE);
	glStencilMask(0xff);
	for (int i = 0; i < STL_LAYERS; i++) {
		GLenum e;
		bool tex_ok = true;

		made++;
		glGenTextures(1, &col[i]);
		glBindTexture(GL_TEXTURE_2D, col[i]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, STL_W, STL_H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glBindTexture(GL_TEXTURE_2D, 0);
		e = glGetError();
		if (e == GL_OUT_OF_MEMORY) {
			notex++;
			tex_ok = false;
		} else if (e != GL_NO_ERROR) {
			pr_err("gl_test: stlayers: GL error 0x%x at layer %d's texture\n", (unsigned)e, i);
			ok = false;
			break;
		}
		glGenRenderbuffers(1, &st[i]);
		glBindRenderbuffer(GL_RENDERBUFFER, st[i]);
		glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, STL_W, STL_H);
		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col[i], 0);
		glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, st[i]);
		if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
			/* a colour texture with no storage leaves it incomplete: the
			 * memory ran out, and no later layer would fit either */
			if (!tex_ok) {
				pr_info("gl_test: stlayers: layer %d has no colour storage: no more layers\n", i);
				break;
			}
			pr_err("gl_test: stlayers: layer %d incomplete\n", i);
			ok = false;
			break;
		}
		complete++;
		glEnable(GL_SCISSOR_TEST);
		glScissor(0, 0, STL_W, STL_H);
		glClearColor(0, 0, 0, 1);
		glClearStencil(0);
		glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		glDisable(GL_SCISSOR_TEST);
		glEnable(GL_STENCIL_TEST);
		glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
		glStencilFunc(GL_ALWAYS, 0, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_INCR);
		stc_triangle(prog, i, STC_RED);
		glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
		glStencilFunc(GL_NOTEQUAL, 0, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		dsk_quad(prog, STC_RED, 0.0f, 0.0f);
		glDisable(GL_STENCIL_TEST);
		e = glGetError();
		if (e == GL_OUT_OF_MEMORY) {
			oom++;
		} else if (e != GL_NO_ERROR) {
			pr_err("gl_test: stlayers: GL error 0x%x at layer %d\n", (unsigned)e, i);
			ok = false;
			break;
		} else {
			drawn[i] = true;
		}
		glFinish();
	}
	pr_info("gl_test: stlayers: %d of %d layer(s) of %dx%d made, alive together; %d refused on the first "
		"pass (GL_OUT_OF_MEMORY), %d texture(s) not allocated\n", complete, STL_LAYERS, STL_W, STL_H, oom,
		notex);
	/* every complete layer again, all of them alive */
	for (int i = 0; ok && i < complete; i++) {
		GLenum e;

		glBindFramebuffer(GL_FRAMEBUFFER, fbo[i]);
		glEnable(GL_STENCIL_TEST);
		glStencilFunc(GL_EQUAL, 1, 0xff);
		glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
		dsk_quad(prog, DSK_GREEN, 0.0f, 0.0f);
		glDisable(GL_STENCIL_TEST);
		glFinish();
		e = glGetError();
		if (e == GL_OUT_OF_MEMORY) {
			oom2++;
			if (drawn[i]) {
				pr_err("gl_test: stlayers: layer %d was drawn, and is refused on the second pass\n", i);
				lost++;
			}
			continue;
		}
		if (e != GL_NO_ERROR) {
			pr_err("gl_test: stlayers: GL error 0x%x at layer %d again\n", (unsigned)e, i);
			ok = false;
			break;
		}
		if (!drawn[i]) {
			/* its first pass never drew: its stencil holds nothing GL defined */
			pr_info("gl_test: stlayers: layer %d refused first, drawn second: not checked\n", i);
			unchecked++;
			continue;
		}
		read++;
		ok &= stl_px(i, "lower left", 60, 60, (i & 1) ? 0xff000000u : DSK_GREEN);
		ok &= stl_px(i, "upper right", STL_W - 61, STL_H - 61, (i & 1) ? DSK_GREEN : 0xff000000u);
	}
	pr_info("gl_test: stlayers: second pass: %d refused (GL_OUT_OF_MEMORY), %d read back, %d drawn layer(s) "
		"refused, %d not checked\n", oom2, read, lost, unchecked);
	ok &= gl_test_check(lost == 0 && unchecked == 0, (uint32_t)(lost + unchecked), 0);
	glStencilFunc(GL_ALWAYS, 0, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glDeleteFramebuffers(STL_LAYERS, fbo);
	for (int i = 0; i < made; i++) {
		glDeleteRenderbuffers(1, &st[i]);
		glDeleteTextures(1, &col[i]);
	}
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- cases 172, 173: a window's depth across a swap and across a split -----
 *
 * EGL 1.5 3.10.1: "The contents of ancillary buffers are always undefined
 * after calling eglSwapBuffers". A backend may drop a window's depth/stencil
 * at the swap (the V3D does; the PowerVR does by default,
 * PVR_BACKEND_OPT_SWAP_DROPS_ZS) and then starts the next frame from the
 * clear values. Undefined contents cannot be asserted, so 172 asserts what
 * each backend defines. A frame flushed WITHOUT a swap (glFinish, a frame the
 * GL core splits) must keep its depth everywhere, which 173 asserts.
 *
 * GL_TEST_SWAP_KEEPS_ZS: 1 where the window's depth survives a swap. */
#if defined(GL_TEST_V3D)
#define GL_TEST_SWAP_KEEPS_ZS 0     /* v3d_surface_swapped: Untouched */
#elif defined(GL_TEST_VIVANTE)
#define GL_TEST_SWAP_KEEPS_ZS 1     /* the GC7000 has no surface_swapped hook */
#elif defined(CONFIG_GL_TEST_PVR_KEEP_ZS)
#define GL_TEST_SWAP_KEEPS_ZS 1     /* the sample's opt-out of PVR_BACKEND_OPT_SWAP_DROPS_ZS */
#else
#define GL_TEST_SWAP_KEEPS_ZS 0     /* PVR_BACKEND_OPT_SWAP_DROPS_ZS on, its default */
#endif

/* A TW x TH XRGB8888 window with a depth buffer, current on its own context. */
static int zs_window(EGLDisplay dpy, struct osal_gpu_mem *wmem, EGLSurface *ws, EGLContext *ctx)
{
	static const EGLint cfg_attribs[] = {
		EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_DEPTH_SIZE, 16, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window win;
	EGLConfig cfg;
	EGLint n = 0, depth = 0;

	release_harness_context();
	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n < 1)
		return -1;
	eglGetConfigAttrib(dpy, cfg, EGL_DEPTH_SIZE, &depth);
	if (!expect_int("a window config with depth", depth >= 16, 1))
		return -1;
	if (osal_gpu_alloc(yakogl_heap_base(GLES_HEAP_DATA), TW * TH * 4u, OSAL_MEM_DATA, wmem) != 0)
		return -1;
	win.phys = wmem->phys;
	win.width = TW;
	win.height = TH;
	win.stride_bytes = TW * 4u;
	win.format = GLES_NATIVE_FORMAT_XRGB8888;
	*ws = eglCreateWindowSurface(dpy, cfg, &win, NULL);
	*ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (*ws == EGL_NO_SURFACE || *ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, *ws, *ws, *ctx))
		return -1;
	return 0;
}

static void zs_window_done(EGLDisplay dpy, struct osal_gpu_mem *wmem, EGLSurface ws, EGLContext ctx)
{
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, ws);
	eglDestroyContext(dpy, ctx);
	eglTerminate(dpy);
	if (g_osal->gpu_free)
		g_osal->gpu_free(wmem);
}

/* A quad over NDC x in [x0, x1], the whole height, at depth z (window depth
 * (z + 1) / 2), in the uniform colour. */
static void zs_quad(GLint loc, float x0, float x1, float z, float r, float g, float b)
{
	struct pc_vertex v[6];
	static const int c[6][2] = { { 0, 0 }, { 1, 0 }, { 1, 1 }, { 0, 0 }, { 1, 1 }, { 0, 1 } };

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = c[i][0] ? x1 : x0;
		v[i].pos[1] = c[i][1] ? 1.0f : -1.0f;
		v[i].pos[2] = z;
		v[i].pos[3] = 1.0f;
		v[i].color[0] = v[i].color[1] = v[i].color[2] = v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glUniform4f(loc, r, g, b, 1.0f);
	glDrawArrays(GL_TRIANGLES, 0, 6);
}

/* 172 swapzs: frame 1 clears depth to 1.0 and draws red over the left half
 * at depth 0.25; swap. Frame 2 clears colour only and draws green over the
 * whole window at depth 0.5, LESS. Where the depth survived the swap, the
 * left half stays black (0.5 is not less than 0.25); where the backend
 * dropped it, frame 2 starts from the clear value 1.0 and the left half is
 * green too. The right half is green either way. */
static int case_swapzs(void)
{
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	struct osal_gpu_mem wmem;
	EGLSurface ws;
	EGLContext ctx;
	GLuint prog;
	GLint loc;
	bool ok = true;

	if (zs_window(dpy, &wmem, &ws, &ctx) != 0)
		return -1;
	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "u_color");
	glViewport(0, 0, TW, TH);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClearDepthf(1.0f);

	/* frame 1 */
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	zs_quad(loc, -1.0f, 0.0f, -0.5f, 1.0f, 0.0f, 0.0f);
	ok &= expect_int("eglSwapBuffers", eglSwapBuffers(dpy, ws), EGL_TRUE);
	read_topdown();
	ok &= expect_px("frame 1: left red", TW / 4, TH / 2, 0xff0000ff, 0);
	ok &= expect_px("frame 1: right black", 3 * TW / 4, TH / 2, 0xff000000, 0);

	/* frame 2: no depth clear */
	glClear(GL_COLOR_BUFFER_BIT);
	zs_quad(loc, -1.0f, 1.0f, 0.0f, 0.0f, 1.0f, 0.0f);
	ok &= expect_int("eglSwapBuffers 2", eglSwapBuffers(dpy, ws), EGL_TRUE);
	if (!check_gl_error("swapzs"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
#if GL_TEST_SWAP_KEEPS_ZS
	ok &= expect_px("frame 2: left kept its depth", TW / 4, TH / 2, 0xff000000, 0);
#else
	ok &= expect_px("frame 2: left from the clear depth", TW / 4, TH / 2, 0xff00ff00, 0);
#endif
	ok &= expect_px("frame 2: right green", 3 * TW / 4, TH / 2, 0xff00ff00, 0);
	zs_window_done(dpy, &wmem, ws, ctx);
	return ok ? 0 : -1;
}

/* 173 splitzs: one frame, split by glFinish: red over the left half at
 * depth 0.25, glFinish (the frame is submitted and continues), then green
 * over the whole window at depth 0.5, LESS. The split keeps the depth:
 * the left half stays red. Then a swap and the same picture read again. */
static int case_splitzs(void)
{
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	struct osal_gpu_mem wmem;
	EGLSurface ws;
	EGLContext ctx;
	GLuint prog;
	GLint loc;
	bool ok = true;

	if (zs_window(dpy, &wmem, &ws, &ctx) != 0)
		return -1;
	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "u_color");
	glViewport(0, 0, TW, TH);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClearDepthf(1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	zs_quad(loc, -1.0f, 0.0f, -0.5f, 1.0f, 0.0f, 0.0f);
	glFinish();
	zs_quad(loc, -1.0f, 1.0f, 0.0f, 0.0f, 1.0f, 0.0f);
	if (!check_gl_error("splitzs"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("split: left kept its depth", TW / 4, TH / 2, 0xff0000ff, 0);
	ok &= expect_px("split: right green", 3 * TW / 4, TH / 2, 0xff00ff00, 0);
	ok &= expect_int("eglSwapBuffers", eglSwapBuffers(dpy, ws), EGL_TRUE);
	read_topdown();
	ok &= expect_px("after the swap: left red", TW / 4, TH / 2, 0xff0000ff, 0);
	ok &= expect_px("after the swap: right green", 3 * TW / 4, TH / 2, 0xff00ff00, 0);
	zs_window_done(dpy, &wmem, ws, ctx);
	return ok ? 0 : -1;
}

/* ---- case 181: a * b + c as one fmad (fusemad) -----------------------------
 *
 * The PowerVR compile makes a sum of a product the sum alone reads one fmad
 * (src/shaderc/online/powervr/prepare.rs fuse_mul_add, blend.rs); the GC7000
 * and the V3D compile the same text without it, and check the case itself.
 * Four quads, every value from a uniform so nothing folds at compile time:
 *   top-left     the fragment shader computes the fused shapes:
 *                x - 2.0 * q (an immediate factor, negated: the shape whose
 *                sign the first fusion dropped, x + 2.0 * q), x - a * b (a
 *                register factor), 2.0 * q + b;
 *   top-right    the vertex shader computes the same into a varying, the
 *                fragment shader writes it;
 *   bottom-left  the controls the fusion must leave alone, in the fragment
 *                shader: a product read twice (x - p and p * 2.0), a product
 *                under abs (x - abs(a * b)), a product under floor at the
 *                sum (floor(a * b * 4.0) + 0.25);
 *   bottom-right the controls in the vertex shader.
 * With x = 0.75, q = 0.125, a = 0.5, b = 0.25 every channel is exact in
 * 8 bits; a dropped negation reads 1.0 where 0.5 is right.
 * ------------------------------------------------------------------------- */

/* Draw `prog` over the rectangle (x0, y0)-(x1, y1) in NDC, its four
 * uniforms u_x, u_q, u_a, u_b set to vals[0..3] (a uniform the program does
 * not use has location -1, which glUniform ignores); false when the
 * program has no a_pos to draw with. */
static bool fma_quad_vals(GLuint prog, float x0, float y0, float x1, float y1, const float vals[4])
{
	const float v[12] = { x0, y0, x1, y0, x0, y1, x0, y1, x1, y0, x1, y1 };
	const char *names[4] = { "u_x", "u_q", "u_a", "u_b" };
	const GLint pos = glGetAttribLocation(prog, "a_pos");

	if (pos < 0) {
		pr_err("gl_test: program %u has no a_pos\n", prog);
		return false;
	}
	glUseProgram(prog);
	for (int i = 0; i < 4; i++)
		glUniform1f(glGetUniformLocation(prog, names[i]), vals[i]);
	glVertexAttribPointer((GLuint)pos, 2, GL_FLOAT, GL_FALSE, 0, v);
	glEnableVertexAttribArray((GLuint)pos);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray((GLuint)pos);
	return true;
}

/* fma_quad_vals with case 181's values: x = 0.75, q = 0.125, a = 0.5, b = 0.25. */
static bool fma_quad(GLuint prog, float x0, float y0, float x1, float y1)
{
	static const float vals[4] = { 0.75f, 0.125f, 0.5f, 0.25f };

	return fma_quad_vals(prog, x0, y0, x1, y1, vals);
}

/* A colour of four values in 0..1 as expect_px reads it (R in the low byte). */
static uint32_t fma_rgba(float r, float g, float b, float a)
{
	const float c[4] = { r, g, b, a };
	uint32_t out = 0;

	for (int i = 0; i < 4; i++)
		out |= (uint32_t)(c[i] * 255.0f + 0.5f) << (8 * i);
	return out;
}

static int case_fusemad(void)
{
	const float x = 0.75f, q = 0.125f, a = 0.5f, b = 0.25f, p = a * b;
	const uint32_t fused = fma_rgba(x - 2.0f * q, x - a * b, 2.0f * q + b, 1.0f);
	const uint32_t ctrl = fma_rgba(x - p, p * 2.0f, x - p, (float)(int)(p * 4.0f) + 0.25f);
	GLuint fs_fused, vs_fused, fs_ctrl, vs_ctrl;
	bool ok = true;

	fs_fused = link_program(gl_test_fma_plain_vs_src, gl_test_fma_fused_fs_src);
	vs_fused = link_program(gl_test_fma_fused_vs_src, gl_test_fma_varying_fs_src);
	fs_ctrl = link_program(gl_test_fma_plain_vs_src, gl_test_fma_ctrl_fs_src);
	vs_ctrl = link_program(gl_test_fma_ctrl_vs_src, gl_test_fma_varying_fs_src);
	if (!fs_fused || !vs_fused || !fs_ctrl || !vs_ctrl)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	/* top is +y in NDC; read_topdown's row 0 is the top */
	if (!fma_quad(fs_fused, -1.0f, 0.0f, 0.0f, 1.0f) || !fma_quad(vs_fused, 0.0f, 0.0f, 1.0f, 1.0f) ||
	    !fma_quad(fs_ctrl, -1.0f, -1.0f, 0.0f, 0.0f) || !fma_quad(vs_ctrl, 0.0f, -1.0f, 1.0f, 0.0f))
		return -1;
	if (!check_gl_error("fusemad"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("fused shapes, fragment", TW / 4, TH / 4, fused, 1);
	ok &= expect_px("fused shapes, vertex", 3 * TW / 4, TH / 4, fused, 1);
	ok &= expect_px("controls, fragment", TW / 4, 3 * TH / 4, ctrl, 1);
	ok &= expect_px("controls, vertex", 3 * TW / 4, 3 * TH / 4, ctrl, 1);
	glDeleteProgram(fs_fused);
	glDeleteProgram(vs_fused);
	glDeleteProgram(fs_ctrl);
	glDeleteProgram(vs_ctrl);
	return ok ? 0 : -1;
}

/* ---- case 182: clamp(x, 0.0, 1.0) as one saturating add (fsat) ------------
 *
 * The PowerVR compile makes a clamp to [0, 1] whose inner bound nothing else
 * reads one `fadd x, 0.0` with the saturate bit, as pco builds NIR's fsat
 * (src/shaderc/online/powervr/prepare.rs fusion_pairs); the GC7000 and the
 * V3D compile the same text their own way, and check the case itself. Four
 * quads, every value from a uniform so nothing folds at compile time:
 *   top-left     the fragment shader computes the folded shapes:
 *                clamp(lo, 0, 1), min(max(in, 0), 1), max(min(hi, 1), 0),
 *                min(1, max(0, hi - 1.1)) (the bounds first);
 *   top-right    the vertex shader computes the same into a varying;
 *   bottom-left  the controls the fold must leave alone, in the fragment
 *                shader: the inner bound read twice (t = max(in, 0) in
 *                min(t, 1) and t * 0.5), an upper bound of 0.8, a lower
 *                bound of 0.2;
 *   bottom-right the controls in the vertex shader.
 * lo = -2.25, in = 0.4, hi = 1.5. Each clamp is written as c * 0.5 + 0.25:
 * an RGBA8 target clamps what the fragment shader writes, so a saturate the
 * fold lost would not show in the clamp itself (-2.25 would read 0 where
 * 0.25 is right, 1.5 would read 1.0 where 0.75 is).
 * ------------------------------------------------------------------------- */

static int case_fsat(void)
{
	/* fma_quad sets u_x, u_q, u_a, u_b; these are lo, in, hi (u_b unused) */
	static const float vals[4] = { -2.25f, 0.4f, 1.5f, 0.0f };
	const uint32_t folded = fma_rgba(0.25f, 0.4f * 0.5f + 0.25f, 0.75f, 0.4f * 0.5f + 0.25f);
	const uint32_t ctrl = fma_rgba(0.4f * 0.5f + 0.25f, 0.2f, 0.8f * 0.5f + 0.25f, 0.2f * 0.5f + 0.25f);
	GLuint fs_folded, vs_folded, fs_ctrl, vs_ctrl;
	bool ok = true;

	fs_folded = link_program(gl_test_fma_plain_vs_src, gl_test_fsat_folded_fs_src);
	vs_folded = link_program(gl_test_fsat_folded_vs_src, gl_test_fma_varying_fs_src);
	fs_ctrl = link_program(gl_test_fma_plain_vs_src, gl_test_fsat_ctrl_fs_src);
	vs_ctrl = link_program(gl_test_fsat_ctrl_vs_src, gl_test_fma_varying_fs_src);
	if (!fs_folded || !vs_folded || !fs_ctrl || !vs_ctrl)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	/* top is +y in NDC; read_topdown's row 0 is the top */
	if (!fma_quad_vals(fs_folded, -1.0f, 0.0f, 0.0f, 1.0f, vals) ||
	    !fma_quad_vals(vs_folded, 0.0f, 0.0f, 1.0f, 1.0f, vals) ||
	    !fma_quad_vals(fs_ctrl, -1.0f, -1.0f, 0.0f, 0.0f, vals) ||
	    !fma_quad_vals(vs_ctrl, 0.0f, -1.0f, 1.0f, 0.0f, vals))
		return -1;
	if (!check_gl_error("fsat"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("folded clamps, fragment", TW / 4, TH / 4, folded, 1);
	ok &= expect_px("folded clamps, vertex", 3 * TW / 4, TH / 4, folded, 1);
	ok &= expect_px("controls, fragment", TW / 4, 3 * TH / 4, ctrl, 1);
	ok &= expect_px("controls, vertex", 3 * TW / 4, 3 * TH / 4, ctrl, 1);
	glDeleteProgram(fs_folded);
	glDeleteProgram(vs_folded);
	glDeleteProgram(fs_ctrl);
	glDeleteProgram(vs_ctrl);
	return ok ? 0 : -1;
}

/* ---- case 183: a half-empty last constant register, drawn often (unipad) --
 *
 * femtovg's programs read `uniform vec2 viewSize` in the vertex shader and
 * end their fragment block on a vec2 too, so the last vec4 register of each
 * stage's constant file holds two dwords of uniform and two of padding. The
 * GC7000's online compile named the padding as uniform dwords past the
 * block, and the backend logged each as missing on every draw: four log
 * lines a draw, 2.7 s of the i.MX8MP cluster's frame 2 in the command-stream
 * step (records/imx8mp/drawstats-2026-10-05; the host reproduction: 800
 * lines in 200 draws of fill_color.frag, 0 for the table's program).
 *
 * 500 small quads of one such program, then glFinish, timed. A draw costs
 * tens of microseconds; the defect costs about a millisecond a log line, so
 * 250 ms tells the two apart by an order of magnitude on every board (it is
 * not a performance target). The colour is read back so the time is the
 * time of draws that drew: (0.25, 0.75, 0, 1) from the two vec2 uniforms.
 * ------------------------------------------------------------------------- */

#define UNIPAD_DRAWS 500
#define UNIPAD_BOUND_MS 250u

static int case_unipad(void)
{
	static const float quad[12] = { -1.0f, -1.0f, 1.0f, -1.0f, -1.0f, 1.0f, -1.0f, 1.0f, 1.0f, -1.0f, 1.0f, 1.0f };
	GLuint prog = link_program(gl_test_unipad_vs_src, gl_test_unipad_fs_src);
	GLint pos;
	uint32_t t0, ms;
	bool ok = true;

	if (!prog)
		return -1;
	pos = glGetAttribLocation(prog, "a_pos");
	if (pos < 0) {
		pr_err("gl_test: unipad: no a_pos\n");
		return -1;
	}
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glUseProgram(prog);
	/* the quad covers the middle half of the target */
	glUniform2f(glGetUniformLocation(prog, "u_scale"), 0.5f, 0.5f);
	glUniform2f(glGetUniformLocation(prog, "u_rg"), 0.25f, 0.75f);
	glVertexAttribPointer((GLuint)pos, 2, GL_FLOAT, GL_FALSE, 0, quad);
	glEnableVertexAttribArray((GLuint)pos);
	glFinish();
	t0 = now_ms();
	for (int i = 0; i < UNIPAD_DRAWS; i++)
		glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	ms = now_ms() - t0;
	glDisableVertexAttribArray((GLuint)pos);
	pr_info("gl_test: unipad: %d draws in %u ms (%u us a draw)\n", UNIPAD_DRAWS, ms,
		ms * 1000u / UNIPAD_DRAWS);
	if (!check_gl_error("unipad"))
		return -1;
	read_topdown();
	ok &= expect_px("the quad", TW / 2, TH / 2, fma_rgba(0.25f, 0.75f, 0.0f, 1.0f), 1);
	ok &= expect_px("outside the quad", 2, 2, fma_rgba(0.0f, 0.0f, 0.0f, 1.0f), 1);
	ok &= expect_int("500 draws under 250 ms", ms < UNIPAD_BOUND_MS, 1);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 184: what a back-to-back dependency costs (depchain) ----------
 *
 * 48 `x = x * u_b + u_c` steps, written as one chain (each step reads the
 * one before), as two chains interleaved (x, y, x, y: each step reads the
 * one two before) and as four (x, y, z, w). The online compile keeps the
 * order (host count of these texts: 49, 48 and 49 groups at
 * nearest-producer distance 1, 2 and 4; 58, 61 and 66 groups, 48 fmads
 * each; 9, 10 and 12 temps). Each arm draws 16
 * full-screen quads into a 1024x600 target, blend off, five timed frames,
 * and logs its 3D busy time. If the one chain costs more per group than the
 * interleaved ones, the USC stalls on a dependency at distance 1 and a
 * scheduler is worth writing (the online fill_color has 56 such pairs, the
 * table's 48); if not, the gap is elsewhere. Values: u_a = u_b = 0.5, u_c =
 * 0.25, so every chain tends to 0.5 and the colour reads about 0x80.
 * ------------------------------------------------------------------------- */

static int case_depchain(void)
{
	enum { W = 1024, H = 600, OVER = 16, REPS = 5 };
	static float quads[OVER * 12];
	static const float quad[12] = { -1.0f, -1.0f, 1.0f, -1.0f, -1.0f, 1.0f, -1.0f, 1.0f, 1.0f, -1.0f, 1.0f, 1.0f };
	const char *const fs[3] = { gl_test_dep1_fs_src, gl_test_dep2_fs_src, gl_test_dep4_fs_src };
	const char *const name[3] = { "one chain (distance 1)", "two chains (distance 2)", "four chains (distance 4)" };
	GLuint tex, fbo, vbo;
	bool ok = true;

	for (int i = 0; i < OVER * 12; i++)
		quads[i] = quad[i % 12];
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: depchain: FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, W, H);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	vbo = upload_vbo(quads, sizeof(quads));
	for (int a = 0; a < 3; a++) {
		GLuint prog = link_program(gl_test_dep_vs_src, fs[a]);
		uint32_t pc0[7], pc1[7], t0, best = 0xffffffffu;
		uint8_t px[4];
		GLint pos;

		if (!prog)
			return -1;
		pos = glGetAttribLocation(prog, "a_pos");
		if (pos < 0)
			return -1;
		glUseProgram(prog);
		glUniform1f(glGetUniformLocation(prog, "u_a"), 0.5f);
		glUniform1f(glGetUniformLocation(prog, "u_b"), 0.5f);
		glUniform1f(glGetUniformLocation(prog, "u_c"), 0.25f);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glVertexAttribPointer((GLuint)pos, 2, GL_FLOAT, GL_FALSE, 0, (const void *)0);
		glEnableVertexAttribArray((GLuint)pos);
		for (int r = 0; r < REPS + 1; r++) {
			glClear(GL_COLOR_BUFFER_BIT);
			if (r == 1)
				gl_test_perf_read(pc0);
			t0 = now_ms();
			glDrawArrays(GL_TRIANGLES, 0, OVER * 6);
			glFinish();
			if (r != 0 && now_ms() - t0 < best)
				best = now_ms() - t0;
		}
		gl_test_perf_read(pc1);
		glDisableVertexAttribArray((GLuint)pos);
		pr_info("gl_test: depchain %d: %s: best %u ms; per frame 3D busy %u us, TA busy %u us\n", a, name[a],
			best, (pc1[1] - pc0[1]) / 800u / REPS, (pc1[0] - pc0[0]) / 800u / REPS);
		glReadPixels(W / 2, H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		/* every chain ends at 0.5 to within a bit (0x80) */
		ok &= expect_int("the colour is the chains' 0.5 (0x7f..0x81)", px[0] >= 0x7f && px[0] <= 0x81, 1);
		glDeleteProgram(prog);
	}
	if (!check_gl_error("depchain"))
		return -1;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glDeleteBuffers(1, &vbo);
	return ok ? 0 : -1;
}

/* ---- case 185: what a group costs by kind (kindcost) ---------------------
 *
 * After 184 (the cost scales with the group count, a dependency at
 * distance 1 is not stalled: 811, 821, 821 us of 3D busy per group per
 * frame), the next question for the fill_color gap: does a group's cost
 * depend on its kind? Each arm repeats one shape, every chain held at 0.5
 * by its uniforms so the colour reads about 0x80:
 *   0 fmul       x = x * u_b; x = x * u_c (u_b 2, u_c 0.5): fmul groups
 *   1 maxmul     x = max(x * u_b, u_c) (0.5, 0.5): fmul + mbyp/mbyp/tst/movc
 *   2 mulneg     x = -x * u_b (u_b 1): fmul, and an mbyp for the negation
 *   3 maxneg     x = max(-x, u_c) (0.5): mbyp/mbyp/tst/movc and an mbyp
 *   4 fadd       x = x + u_c; x = x - u_d (equal): fadd groups
 *   5 fmad blend 184's one chain with femtovg's blend (ONE,
 *                ONE_MINUS_SRC_ALPHA): its ordered tail against arm 6
 *   6 fmad       184's one chain, blend off
 * Host counts, online PowerVR compile of these texts, are in the commit
 * that adds the case. Per arm: 16 full-screen quads into 1024x600, five
 * timed frames, 3D busy logged as 184 logs it.
 * ------------------------------------------------------------------------- */

static int case_kindcost(void)
{
	enum { W = 1024, H = 600, OVER = 16, REPS = 5, ARMS = 7 };
	static float quads[OVER * 12];
	static const float quad[12] = { -1.0f, -1.0f, 1.0f, -1.0f, -1.0f, 1.0f, -1.0f, 1.0f, 1.0f, -1.0f, 1.0f, 1.0f };
	static const struct {
		const char *name, *fs;
		float u[4]; /* u_a, u_b, u_c, u_d */
		bool blend;
	} arm[ARMS] = {
		{ "fmul", gl_test_kc_fmul_fs_src, { 0.5f, 2.0f, 0.5f, 0.0f }, false },
		{ "maxmul (fmul + tst/movc)", gl_test_kc_maxmul_fs_src, { 0.5f, 0.5f, 0.5f, 0.0f }, false },
		{ "mulneg (fmul + mbyp)", gl_test_kc_mulneg_fs_src, { 0.5f, 1.0f, 0.0f, 0.0f }, false },
		{ "maxneg (tst/movc + mbyp)", gl_test_kc_maxneg_fs_src, { 0.5f, 0.0f, 0.5f, 0.0f }, false },
		{ "fadd", gl_test_kc_fadd_fs_src, { 0.5f, 0.0f, 0.25f, 0.25f }, false },
		{ "fmad, femtovg's blend", gl_test_dep1_fs_src, { 0.5f, 0.5f, 0.25f, 0.0f }, true },
		{ "fmad, blend off", gl_test_dep1_fs_src, { 0.5f, 0.5f, 0.25f, 0.0f }, false },
	};
	static const char *const uname[4] = { "u_a", "u_b", "u_c", "u_d" };
	GLuint tex, fbo, vbo;
	bool ok = true;

	for (int i = 0; i < OVER * 12; i++)
		quads[i] = quad[i % 12];
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: kindcost: FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, W, H);
	glDisable(GL_DEPTH_TEST);
	glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
	vbo = upload_vbo(quads, sizeof(quads));
	for (int a = 0; a < ARMS; a++) {
		GLuint prog = link_program(gl_test_dep_vs_src, arm[a].fs);
		uint32_t pc0[7], pc1[7], t0, best = 0xffffffffu;
		uint8_t px[4];
		GLint pos;

		if (!prog)
			return -1;
		pos = glGetAttribLocation(prog, "a_pos");
		if (pos < 0)
			return -1;
		glUseProgram(prog);
		/* a uniform an arm does not read has location -1, which glUniform ignores */
		for (int k = 0; k < 4; k++)
			glUniform1f(glGetUniformLocation(prog, uname[k]), arm[a].u[k]);
		if (arm[a].blend)
			glEnable(GL_BLEND);
		else
			glDisable(GL_BLEND);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glVertexAttribPointer((GLuint)pos, 2, GL_FLOAT, GL_FALSE, 0, (const void *)0);
		glEnableVertexAttribArray((GLuint)pos);
		for (int r = 0; r < REPS + 1; r++) {
			/* black, alpha 0: the blend arm's ONE_MINUS_SRC_ALPHA leaves the
			 * opaque source as it is, so every arm reads its chain's 0.5 */
			glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			if (r == 1)
				gl_test_perf_read(pc0);
			t0 = now_ms();
			glDrawArrays(GL_TRIANGLES, 0, OVER * 6);
			glFinish();
			if (r != 0 && now_ms() - t0 < best)
				best = now_ms() - t0;
		}
		gl_test_perf_read(pc1);
		glDisableVertexAttribArray((GLuint)pos);
		pr_info("gl_test: kindcost %d: %s: best %u ms; per frame 3D busy %u us, TA busy %u us\n", a, arm[a].name,
			best, (pc1[1] - pc0[1]) / 800u / REPS, (pc1[0] - pc0[0]) / 800u / REPS);
		glReadPixels(W / 2, H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		ok &= expect_int("the colour is the chain's 0.5 (0x7f..0x81)", px[0] >= 0x7f && px[0] <= 0x81, 1);
		glDeleteProgram(prog);
	}
	glDisable(GL_BLEND);
	if (!check_gl_error("kindcost"))
		return -1;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glDeleteBuffers(1, &vbo);
	return ok ? 0 : -1;
}

/* ---- case 186: case 39's fill_color split by feature (fcsplit) ----------
 *
 * 184 and 185 showed a group costs about the same whatever its dependency
 * distance or kind, so the online fill_color (136 groups) should not be
 * slower than the table's (140), yet it is (101.6 against 83.4 ms in case
 * 39). What those cases lack and fill_color has: the discard, femtovg's
 * blend (the ordered tail reading the pixel), the varying iteration and
 * the uniform block. This splits two of them off with programs the table
 * holds rows for: fill_color and fill_color.nodiscard (the same source
 * without its `discard`), each blended (key 0x98661, case 39's) and with
 * blend off (key 0). Uniforms as case 39's: strokeThr -1, so the discard
 * never fires, and both programs draw the same picture.
 * Read in an online image against a table image, per arm:
 *   the ratio falls to about 1 without the discard: the discard path;
 *   it falls with blend off: the ordered tail / the pixel read;
 *   it stays about 1.2 in all four: the common body (the next split is
 *   inside it).
 * ------------------------------------------------------------------------- */
/* Case 186's body, with scissorRadius = `radius` (frag[12].w) and its log
 * lines tagged `tag`: 0 takes fill_color's simpler scissor arm (case 39's),
 * a positive radius the sdroundrect arm (case 187). */
static int fcsplit_run(float radius, const char *tag)
{
	enum { W = 1024, H = 600, OVER = 16, REPS = 5, ARMS = 4 };
	struct fvg_vertex { float xy[2]; float uv[2]; };
	static struct fvg_vertex quads[OVER * 6];
	static const float xs[6] = { 0, W, W, 0, W, 0 }, ys[6] = { 0, 0, H, 0, H, H };
	static const struct {
		const char *name;
		bool nodiscard, blend;
	} arm[ARMS] = {
		{ "fill_color, femtovg's blend", false, true },
		{ "fill_color.nodiscard, femtovg's blend", true, true },
		{ "fill_color, blend off", false, false },
		{ "fill_color.nodiscard, blend off", true, false },
	};
	float frag[56] = { 0 };
	uint32_t got[ARMS];
	GLuint tex, fbo, vbo, vs;
	bool ok = true;

	for (int i = 0; i < OVER * 6; i++) {
		quads[i].xy[0] = xs[i % 6];
		quads[i].xy[1] = ys[i % 6];
		quads[i].uv[0] = 0.5f;
		quads[i].uv[1] = 1.0f;
	}
	/* case 39's uniforms: innerCol translucent red, the scissor open, the
	 * stroke at full coverage, strokeThr -1 (the discard never fires) */
	frag[6 * 4 + 0] = 0.3f; frag[6 * 4 + 3] = 0.5f;
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;
	frag[10 * 4 + 0] = 1.0f;
	frag[10 * 4 + 1] = -1.0f;
	frag[10 * 4 + 3] = 5.0f;
	frag[12 * 4 + 3] = radius;
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: %s: FBO incomplete\n", tag);
		return -1;
	}
	glViewport(0, 0, W, H);
	glDisable(GL_DEPTH_TEST);
	glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
	vbo = upload_vbo(quads, sizeof(quads));
	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	if (!vs)
		return -1;
	for (int a = 0; a < ARMS; a++) {
		GLuint fs = compile(GL_FRAGMENT_SHADER,
				    arm[a].nodiscard ? gl_test_fvg_fill_color_nodiscard_fs_src : gl_test_fvg_fill_color_fs_src);
		GLuint prog = glCreateProgram();
		GLint linked = 0;
		uint32_t pc0[7], pc1[7], t0, best = 0xffffffffu;
		uint8_t px[4];

		if (!fs)
			return -1;
		glAttachShader(prog, vs);
		glAttachShader(prog, fs);
		glBindAttribLocation(prog, 0, "vertex");
		glBindAttribLocation(prog, 1, "tcoord");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &linked);
		if (!linked)
			return -1;
		glUseProgram(prog);
		glUniform2f(glGetUniformLocation(prog, "viewSize"), (float)W, (float)H);
		glUniform4fv(glGetUniformLocation(prog, "frag"), 14, frag);
		if (arm[a].blend)
			glEnable(GL_BLEND);
		else
			glDisable(GL_BLEND);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		for (int r = 0; r < REPS + 1; r++) {
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			if (r == 1)
				gl_test_perf_read(pc0);
			t0 = now_ms();
			glDrawArrays(GL_TRIANGLES, 0, OVER * 6);
			glFinish();
			if (r != 0 && now_ms() - t0 < best)
				best = now_ms() - t0;
		}
		gl_test_perf_read(pc1);
		glDisableVertexAttribArray(0);
		glDisableVertexAttribArray(1);
		pr_info("gl_test: %s %d: %s: best %u ms; per frame 3D busy %u us, TA busy %u us\n", tag, a, arm[a].name,
			best, (pc1[1] - pc0[1]) / 800u / REPS, (pc1[0] - pc0[0]) / 800u / REPS);
		glReadPixels(W / 2, H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		got[a] = (uint32_t)px[0] | (uint32_t)px[1] << 8 | (uint32_t)px[2] << 16 | (uint32_t)px[3] << 24;
		glDeleteProgram(prog);
		glDeleteShader(fs);
	}
	glDisable(GL_BLEND);
	if (!check_gl_error(tag))
		return -1;
	pr_info("gl_test: %s pixels: %08x %08x %08x %08x\n", tag, got[0], got[1], got[2], got[3]);
	/* the discard never fires, so each pair draws one picture; blend off
	 * leaves innerCol's red, 0.3 (0x4c), where the scissor mask is 1 (radius
	 * 0; the sdroundrect arm scales it), and the blend stacks 16 of them */
	ok &= expect_int("fill_color and its nodiscard twin agree, blended", got[0] == got[1], 1);
	ok &= expect_int("fill_color and its nodiscard twin agree, blend off", got[2] == got[3], 1);
	if (radius == 0.0f)
		ok &= expect_int("blend off: innerCol's red (0x4b..0x4d)", (got[2] & 0xff) >= 0x4b && (got[2] & 0xff) <= 0x4d, 1);
	ok &= expect_int("the blend stacks the layers (red past blend off's)", (got[0] & 0xff) > (got[2] & 0xff), 1);
	glDeleteShader(vs);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glDeleteBuffers(1, &vbo);
	return ok ? 0 : -1;
}

static int case_fcsplit(void)
{
	return fcsplit_run(0.0f, "fcsplit");
}

/* ---- case 187: 186 with the other scissor arm (predarm) --------------------
 *
 * 186 found the table's fill_color groups about 30% cheaper than ours at an
 * equal count (105 each, nodiscard + blend: 54854 against 77650 us). The
 * table (pco_cf.c lower_if_pred_exec) runs scissorMask's
 * `if (scissorRadius > 0.0)` as two predicated arms: groups 32..63 under
 * if(!p0) (the sdroundrect arm, 32 groups) and 64..85 under if(p0) (the
 * plain arm, 22) of the nodiscard + blend program; ours computes both arms and selects (89 unconditional
 * groups). If a predicated-off group costs about nothing, the table runs
 * about 73 groups of its 105 at radius 0, which is what 186 measured (74
 * at our per-group cost). The same case with scissorRadius 4 takes the
 * 32-group arm: the table should then cost about 10 groups more (about
 * +7 ms on the nodiscard + blend arm), ours the same as at radius 0.
 * ------------------------------------------------------------------------- */
static int case_predarm(void)
{
	return fcsplit_run(4.0f, "predarm");
}

/* ---- case 188: an if/else with arithmetic arms, each side (predif) -------
 *
 * The PowerVR compile runs an `if` whose arms are arithmetic predicated:
 * the condition's tstz.p0, the then arm under if(!p0), the else arm under
 * if(p0), and the joins (selects on the same condition) after; an arm that
 * runs in no instance costs about nothing (gl_test 186/187, the fill_color
 * gap). This reads back each arm, each way the condition can be met:
 *   top row     the `if` on a uniform in the fragment shader, drawn with
 *               u_x = 1 (left, the then arm) and u_x = -1 (right, else);
 *   middle row  the same `if` in the vertex shader, into a varying;
 *   bottom row  the same `if` on gl_FragCoord.x in the fragment shader:
 *               one draw whose left half takes the then arm and right half
 *               the else arm, so instances in one task go both ways.
 * u_q = 0.125, u_a = 0.25: then (0.5, 0.625, 0.25, 1), else (0.125, 0.125,
 * 0.75, 1), exact in 8 bits.
 * ------------------------------------------------------------------------- */

static int case_predif(void)
{
	static const float then_vals[4] = { 1.0f, 0.125f, 0.25f, 0.0f };
	static const float else_vals[4] = { -1.0f, 0.125f, 0.25f, 0.0f };
	const uint32_t then_c = fma_rgba(0.5f, 0.625f, 0.25f, 1.0f);
	const uint32_t else_c = fma_rgba(0.125f, 0.125f, 0.75f, 1.0f);
	GLuint fs_uni, vs_uni, fs_coord;
	bool ok = true;

	fs_uni = link_program(gl_test_fma_plain_vs_src, gl_test_predif_uni_fs_src);
	vs_uni = link_program(gl_test_predif_uni_vs_src, gl_test_fma_varying_fs_src);
	fs_coord = link_program(gl_test_fma_plain_vs_src, gl_test_predif_coord_fs_src);
	if (!fs_uni || !vs_uni || !fs_coord)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	/* rows in NDC, top is +y: [1/3, 1], [-1/3, 1/3], [-1, -1/3] */
	if (!fma_quad_vals(fs_uni, -1.0f, 1.0f / 3.0f, 0.0f, 1.0f, then_vals) ||
	    !fma_quad_vals(fs_uni, 0.0f, 1.0f / 3.0f, 1.0f, 1.0f, else_vals) ||
	    !fma_quad_vals(vs_uni, -1.0f, -1.0f / 3.0f, 0.0f, 1.0f / 3.0f, then_vals) ||
	    !fma_quad_vals(vs_uni, 0.0f, -1.0f / 3.0f, 1.0f, 1.0f / 3.0f, else_vals) ||
	    !fma_quad_vals(fs_coord, -1.0f, -1.0f, 1.0f, -1.0f / 3.0f, then_vals))
		return -1;
	if (!check_gl_error("predif"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("uniform, fragment, then", TW / 4, TH / 6, then_c, 1);
	ok &= expect_px("uniform, fragment, else", 3 * TW / 4, TH / 6, else_c, 1);
	ok &= expect_px("uniform, vertex, then", TW / 4, TH / 2, then_c, 1);
	ok &= expect_px("uniform, vertex, else", 3 * TW / 4, TH / 2, else_c, 1);
	ok &= expect_px("per pixel, then (x < 32)", TW / 4, 5 * TH / 6, then_c, 1);
	ok &= expect_px("per pixel, else (x >= 32)", 3 * TW / 4, 5 * TH / 6, else_c, 1);
	ok &= expect_px("per pixel, the last then column", TW / 2 - 1, 5 * TH / 6, then_c, 1);
	ok &= expect_px("per pixel, the first else column", TW / 2, 5 * TH / 6, else_c, 1);
	glDeleteProgram(fs_uni);
	glDeleteProgram(vs_uni);
	glDeleteProgram(fs_coord);
	return ok ? 0 : -1;
}

/* ---- case 189: a product by the constant 1.0 (mulone) ---------------------
 *
 * `x * 1.0` is x on every input, so the PowerVR compile reads x where the
 * product is read and spends no group on it (fill_color's
 * `scissorMat * vec3(p, 1.0)`). This reads back each side:
 *   top row     M * vec3(0.5, 0.5, 1.0): the third column read as it is,
 *               one of its lanes a negated uniform;
 *   middle row  M * vec3(0.5): no lane is 1.0, every product is kept;
 *   bottom row  abs(), a negation and a plain uniform, each times 1.0.
 * M = (u_q, u_a, 0 | u_a, u_a, u_q | u_b, -u_q, u_x); u_x = 0.25,
 * u_q = 0.125, u_a = 0.25, u_b = 0.25: top (0.4375, 0.125, 0.3125), middle
 * (0.3125, 0.1875, 0.1875), bottom (0.25, 0.125, 0.25).
 * ------------------------------------------------------------------------- */

static int case_mulone(void)
{
	static const float vals[4] = { 0.25f, 0.125f, 0.25f, 0.25f };
	const uint32_t one_c = fma_rgba(0.4375f, 0.125f, 0.3125f, 1.0f);
	const uint32_t half_c = fma_rgba(0.3125f, 0.1875f, 0.1875f, 1.0f);
	const uint32_t mods_c = fma_rgba(0.25f, 0.125f, 0.25f, 1.0f);
	GLuint one, half, mods;
	bool ok = true;

	one = link_program(gl_test_fma_plain_vs_src, gl_test_mulone_one_fs_src);
	half = link_program(gl_test_fma_plain_vs_src, gl_test_mulone_half_fs_src);
	mods = link_program(gl_test_fma_plain_vs_src, gl_test_mulone_mods_fs_src);
	if (!one || !half || !mods)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	/* rows in NDC, top is +y: [1/3, 1], [-1/3, 1/3], [-1, -1/3] */
	if (!fma_quad_vals(one, -1.0f, 1.0f / 3.0f, 1.0f, 1.0f, vals) ||
	    !fma_quad_vals(half, -1.0f, -1.0f / 3.0f, 1.0f, 1.0f / 3.0f, vals) ||
	    !fma_quad_vals(mods, -1.0f, -1.0f, 1.0f, -1.0f / 3.0f, vals))
		return -1;
	if (!check_gl_error("mulone"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("M * vec3(0.5, 0.5, 1.0)", TW / 2, TH / 6, one_c, 1);
	ok &= expect_px("M * vec3(0.5), no lane 1.0", TW / 2, TH / 2, half_c, 1);
	ok &= expect_px("abs, negation, uniform, each * 1.0", TW / 2, 5 * TH / 6, mods_c, 1);
	glDeleteProgram(one);
	glDeleteProgram(half);
	glDeleteProgram(mods);
	return ok ? 0 : -1;
}

/* ---- case 190: a blend factor computed once (blendonce) -----------------
 *
 * The PowerVR blends in the shader; a factor that is the same value in
 * every channel (1 - As, 1 - Ad) is computed once and read by the later
 * channels. The pixel is (0.25, 0.5, 0.75, 0.5), non-zero in every channel
 * so that a channel reading a wrong factor shows; the source
 * (0.75, 0.5, 0.25, 0.25). Four columns, each a quad under one state:
 *   1  SRC_ALPHA, ONE_MINUS_SRC_ALPHA for colour and alpha: every channel
 *      reads the one 1 - As      -> (0.375, 0.5, 0.625, 0.4375)
 *   2  the same, red masked: green is the first to compute it
 *                                -> (0.25, 0.5, 0.625, 0.4375)
 *   3  the same factors, MIN on the colour (no factor) and ADD on alpha:
 *      alpha computes it alone   -> (0.25, 0.5, 0.25, 0.4375)
 *   4  ONE_MINUS_DST_ALPHA, ONE_MINUS_SRC_ALPHA for colour, ONE, ZERO for
 *      alpha: both factors shared -> (0.5625, 0.625, 0.6875, 0.25)
 * GL_MIN is ES 3.0's (EXT_blend_minmax); YakoGL takes it on every GPU
 * (src/gl/state.rs glBlendEquationSeparate), so column 3 is always drawn
 * and a refusal is a GL error here.
 * ------------------------------------------------------------------------- */

static int case_blendonce(void)
{
	static const float src[4] = { 0.75f, 0.5f, 0.25f, 0.25f };
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_fma_plain_vs_src, gl_test_blendonce_fs_src);
	if (!prog)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.25f, 0.5f, 0.75f, 0.5f);
	glClear(GL_COLOR_BUFFER_BIT);
	glEnable(GL_BLEND);
	glBlendEquation(GL_FUNC_ADD);
	glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
	if (!fma_quad_vals(prog, -1.0f, -1.0f, -0.5f, 1.0f, src))
		return -1;
	glColorMask(GL_FALSE, GL_TRUE, GL_TRUE, GL_TRUE);
	if (!fma_quad_vals(prog, -0.5f, -1.0f, 0.0f, 1.0f, src))
		return -1;
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glBlendEquationSeparate(0x8007 /* GL_MIN */, GL_FUNC_ADD);
	if (!fma_quad_vals(prog, 0.0f, -1.0f, 0.5f, 1.0f, src))
		return -1;
	glBlendEquation(GL_FUNC_ADD);
	glBlendFuncSeparate(GL_ONE_MINUS_DST_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ZERO);
	if (!fma_quad_vals(prog, 0.5f, -1.0f, 1.0f, 1.0f, src))
		return -1;
	glDisable(GL_BLEND);
	if (!check_gl_error("blendonce"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("1 - As in every channel", TW / 8, TH / 2, fma_rgba(0.375f, 0.5f, 0.625f, 0.4375f), 2);
	ok &= expect_px("red masked, green computes it", 3 * TW / 8, TH / 2,
			fma_rgba(0.25f, 0.5f, 0.625f, 0.4375f), 2);
	ok &= expect_px("MIN on colour, alpha computes it", 5 * TW / 8, TH / 2,
			fma_rgba(0.25f, 0.5f, 0.25f, 0.4375f), 2);
	ok &= expect_px("1 - Ad and 1 - As together", 7 * TW / 8, TH / 2,
			fma_rgba(0.5625f, 0.625f, 0.6875f, 0.25f), 2);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 191: products of a varying fused with their sum (varyfma) -------
 *
 * In a fragment shader an iterated varying sits in the temporary its
 * iteration pinned, which nothing else writes: a product of it and the sum
 * that alone reads the product are one fmad on the PowerVR, as in pco's
 * program (fill_color's strokeMask, `ftcoord.x * 2.0 - 1.0`). The vertex
 * shader writes v = (u_x, u_q, u_a, u_b) at every vertex, so the varying is
 * exact: u_x = 0.75, u_q = 0.5 gives (0.75 * 2 - 1, 0.5 * 0.5 + 0.25,
 * 1 - 0.75 * 0.5, 1) = (0.5, 0.5, 0.625, 1).
 * ------------------------------------------------------------------------- */

static int case_varyfma(void)
{
	static const float vals[4] = { 0.75f, 0.5f, 0.0f, 0.0f };
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_varyfma_vs_src, gl_test_varyfma_fs_src);
	if (!prog)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	if (!fma_quad_vals(prog, -1.0f, -1.0f, 1.0f, 1.0f, vals))
		return -1;
	if (!check_gl_error("varyfma"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("v.x * 2 - 1, v.y * 0.5 + 0.25, 1 - v.x * v.y", TW / 2, TH / 2,
			fma_rgba(0.5f, 0.5f, 0.625f, 1.0f), 1);
	ok &= expect_px("the same at a corner", 1, 1, fma_rgba(0.5f, 0.5f, 0.625f, 1.0f), 1);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 192: values live across a predicated if (predlive) --------------
 *
 * The PowerVR runs an `if` with arithmetic arms predicated and writes a
 * value both arms compute to one register from both arms (no select after
 * the `if`, as in pco's program). Two values from before the `if` (a, b)
 * are read in both arms and after it, so their registers are live across
 * the arms beside the joined ones (r, s):
 *   a = u_q * 3, b = u_a + 0.125;
 *   then: r = a * 2, s = b - a;  else: r = b * 0.5, s = a + b;
 *   colour (r, s, a, b).
 * u_q = 0.125, u_a = 0.5: a = 0.375, b = 0.625; then (0.75, 0.25, 0.375,
 * 0.625), else (0.3125, 1.0, 0.375, 0.625). Rows as case 188's: the `if`
 * on u_x drawn with u_x = 1 (left, then) and -1 (right, else), and the
 * `if` on gl_FragCoord.x < 32 (instances of one draw take both arms).
 * ------------------------------------------------------------------------- */

static int case_predlive(void)
{
	static const float then_vals[4] = { 1.0f, 0.125f, 0.5f, 0.0f };
	static const float else_vals[4] = { -1.0f, 0.125f, 0.5f, 0.0f };
	const uint32_t then_c = fma_rgba(0.75f, 0.25f, 0.375f, 0.625f);
	const uint32_t else_c = fma_rgba(0.3125f, 1.0f, 0.375f, 0.625f);
	GLuint uni, coord;
	bool ok = true;

	uni = link_program(gl_test_fma_plain_vs_src, gl_test_predlive_uni_fs_src);
	coord = link_program(gl_test_fma_plain_vs_src, gl_test_predlive_coord_fs_src);
	if (!uni || !coord)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	/* rows in NDC, top is +y: [0, 1] the uniform, [-1, 0] the coordinate */
	if (!fma_quad_vals(uni, -1.0f, 0.0f, 0.0f, 1.0f, then_vals) ||
	    !fma_quad_vals(uni, 0.0f, 0.0f, 1.0f, 1.0f, else_vals) ||
	    !fma_quad_vals(coord, -1.0f, -1.0f, 1.0f, 0.0f, then_vals))
		return -1;
	if (!check_gl_error("predlive"))
		return -1;
	read_topdown();
	dump_map(s_px, TW, TH);
	ok &= expect_px("uniform, then", TW / 4, TH / 4, then_c, 1);
	ok &= expect_px("uniform, else", 3 * TW / 4, TH / 4, else_c, 1);
	ok &= expect_px("per pixel, then (x < 32)", TW / 4, 3 * TH / 4, then_c, 1);
	ok &= expect_px("per pixel, else (x >= 32)", 3 * TW / 4, 3 * TH / 4, else_c, 1);
	ok &= expect_px("per pixel, the last then column", TW / 2 - 1, 3 * TH / 4, then_c, 1);
	ok &= expect_px("per pixel, the first else column", TW / 2, 3 * TH / 4, else_c, 1);
	glDeleteProgram(uni);
	glDeleteProgram(coord);
	return ok ? 0 : -1;
}

/* ---- case 193: a predicated-off arm grown by N groups (offarm) -----------
 *
 * 186/187 suggest that a group no instance runs costs about nothing on the
 * table's fill_color, and leave an old gap of about 5.5 ms in 186 arm 1 (on
 * afa12e9b) that the group counts do not explain. This asks the question
 * directly: 186's arm 1 (fill_color.nodiscard, femtovg's blend, case 39's
 * uniforms, scissorRadius 0) with scissorMask's `if (scissorRadius > 0.0)`
 * arm -- the one radius 0 never runs -- grown by N dependent steps
 * `distance = distance * frag[13].y + frag[13].z` (tools/shaders/offarm/;
 * N = 0 is fill_color.nodiscard itself). The arm the pixels run is the
 * same in every program. Host count, online compile, blend key 0x98661
 * (tests/usc_limits.rs gl_test_case_193_off_arm_groups_on_the_host):
 *   N      0    8   32  128
 *   groups 104  114  138  234   (30 temps each)
 *   off arm 31   41   65  161   (under if(!p0); the taken arm 23)
 * the off arm grows by the N fmad and two copies of the steps' uniforms,
 * and every group outside it is N = 0's. The table's rows (pco_shc):
 *   groups 105  115  139  235   (14 temps each)
 *   off arm 32   42   66  162   (predicated too, no branch; taken arm 22)
 * the off arm grows the same way (the N fmad and two mbyp); both
 * programs run 73 groups outside it.
 *   3D busy flat in N: an off group costs nothing (the old gap is
 *   elsewhere); rising with N: the slope is what one costs, against an
 *   executed group's (184/185: about 0.8 ms a frame here).
 * Run in an online image and in a table image (rows from
 * tools/glsl2usc.sh with tools/shaders/offarm/): a program without a
 * shader is logged by name and fails the case after the others are timed.
 * frag[13].y = 1, .z = 0: the steps are the identity, so a positive radius
 * would draw what fill_color.nodiscard draws.
 * ------------------------------------------------------------------------- */
/* One program of 193/194: N steps in the off arm, its source and its file. */
struct offarm_arm {
	int n;
	const char *file, *fs;
};

/* 193's body over `arms` programs (at most 16), its log lines tagged `tag`. */
static int offarm_run(const struct offarm_arm *arm, int arms, const char *tag)
{
	enum { W = 1024, H = 600, OVER = 16, REPS = 5, MAX_ARMS = 16 };
	struct fvg_vertex { float xy[2]; float uv[2]; };
	static struct fvg_vertex quads[OVER * 6];
	static const float xs[6] = { 0, W, W, 0, W, 0 }, ys[6] = { 0, 0, H, 0, H, H };
	float frag[56] = { 0 };
	uint32_t got[MAX_ARMS] = { 0 };
	bool ran[MAX_ARMS] = { false };
	GLuint tex, fbo, vbo, vs;
	int missing = 0;
	bool ok = true;

	if (arms < 1 || arms > MAX_ARMS)
		return -1;
	for (int i = 0; i < OVER * 6; i++) {
		quads[i].xy[0] = xs[i % 6];
		quads[i].xy[1] = ys[i % 6];
		quads[i].uv[0] = 0.5f;
		quads[i].uv[1] = 1.0f;
	}
	/* case 39's uniforms (186's at radius 0), and the steps' identity */
	frag[6 * 4 + 0] = 0.3f; frag[6 * 4 + 3] = 0.5f;
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;
	frag[10 * 4 + 0] = 1.0f;
	frag[10 * 4 + 1] = -1.0f;
	frag[10 * 4 + 3] = 5.0f;
	frag[13 * 4 + 1] = 1.0f;
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: %s: FBO incomplete\n", tag);
		return -1;
	}
	glViewport(0, 0, W, H);
	glDisable(GL_DEPTH_TEST);
	glEnable(GL_BLEND);
	glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
	vbo = upload_vbo(quads, sizeof(quads));
	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	if (!vs)
		return -1;
	for (int a = 0; a < arms; a++) {
		GLuint fs = compile(GL_FRAGMENT_SHADER, arm[a].fs);
		GLuint prog;
		GLint linked = 0;
		uint32_t pc0[7], pc1[7], t0, best = 0xffffffffu;
		uint8_t px[4];

		if (!fs) {
			/* a table image without the variant's rows: time the rest */
			pr_err("gl_test: %s %d: no shader for %s (+%d groups)\n", tag, a, arm[a].file, arm[a].n);
			missing++;
			continue;
		}
		prog = glCreateProgram();
		glAttachShader(prog, vs);
		glAttachShader(prog, fs);
		glBindAttribLocation(prog, 0, "vertex");
		glBindAttribLocation(prog, 1, "tcoord");
		glLinkProgram(prog);
		glGetProgramiv(prog, GL_LINK_STATUS, &linked);
		if (!linked)
			return -1;
		glUseProgram(prog);
		glUniform2f(glGetUniformLocation(prog, "viewSize"), (float)W, (float)H);
		glUniform4fv(glGetUniformLocation(prog, "frag"), 14, frag);
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
		glEnableVertexAttribArray(0);
		glEnableVertexAttribArray(1);
		for (int r = 0; r < REPS + 1; r++) {
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			if (r == 1)
				gl_test_perf_read(pc0);
			t0 = now_ms();
			glDrawArrays(GL_TRIANGLES, 0, OVER * 6);
			glFinish();
			if (r != 0 && now_ms() - t0 < best)
				best = now_ms() - t0;
		}
		gl_test_perf_read(pc1);
		glDisableVertexAttribArray(0);
		glDisableVertexAttribArray(1);
		pr_info("gl_test: %s %d: off arm +%d groups: best %u ms; per frame 3D busy %u us, TA busy %u us\n", tag,
			a, arm[a].n, best, (pc1[1] - pc0[1]) / 800u / REPS, (pc1[0] - pc0[0]) / 800u / REPS);
		glReadPixels(W / 2, H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		got[a] = (uint32_t)px[0] | (uint32_t)px[1] << 8 | (uint32_t)px[2] << 16 | (uint32_t)px[3] << 24;
		ran[a] = true;
		glDeleteProgram(prog);
		glDeleteShader(fs);
	}
	glDisable(GL_BLEND);
	if (!check_gl_error(tag))
		return -1;
	pr_info("gl_test: %s pixels:", tag);
	for (int a = 0; a < arms; a++)
		pr_info(" %08x", got[a]);
	pr_info("\n");
	ok &= expect_int("every program has its shader", missing, 0);
	/* the off arm never runs, so every N draws N = 0's picture: innerCol's
	 * red 0.3 under the blend, 16 layers stacked past one layer's 0x4c */
	for (int a = 1; a < arms; a++)
		if (ran[0] && ran[a])
			ok &= expect_int("the off arm leaves the picture as N = 0 draws it", got[a] == got[0], 1);
	if (ran[0])
		ok &= expect_int("the blend stacks the layers (red past 0x4c)", (got[0] & 0xff) > 0x4c, 1);
	glDeleteShader(vs);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glDeleteBuffers(1, &vbo);
	return ok ? 0 : -1;
}

static int case_offarm(void)
{
	static const struct offarm_arm arm[] = {
		{ 0, "femtovg/fill_color.nodiscard.frag", gl_test_fvg_fill_color_nodiscard_fs_src },
		{ 8, "offarm/fill_color.nodiscard.offarm8.frag", gl_test_offarm8_fs_src },
		{ 32, "offarm/fill_color.nodiscard.offarm32.frag", gl_test_offarm32_fs_src },
		{ 128, "offarm/fill_color.nodiscard.offarm128.frag", gl_test_offarm128_fs_src },
	};

	return offarm_run(arm, (int)(sizeof(arm) / sizeof(arm[0])), "offarm");
}

/* ---- case 194: 193 over a finer N series (offarmn) ---------------------------
 *
 * 193 on the AM62P (tools/cts/RESULTS.md): a predicated-off group costs
 * +0.107 ms a frame at N = 8, +0.149 at 32, +0.286 at 128 (the table
 * +0.145, +0.181, +0.288), rising with N, where an executed group costs
 * about 0.8 ms. Is that a step and a slope, a fixed cost per arm and one
 * per group? The same program at 14 N, 193's body: 0, 2, 4, 8, 12, 16, 24,
 * 32, 48, 64, 96, 128, 192, 256 (tools/gen_offarm.py writes the sources).
 * Host counts per N, online and table, in tests/usc_limits.rs
 * gl_test_case_194_off_arm_series_on_the_host: the off arm is 31 groups
 * online and 32 in the table, plus N + 2 for N > 0 (the N fmad and two
 * mbyp) in both; the 73 groups outside it are the same at every N. Online
 * 104 to 362 groups, the table 105 to 363; N = 256 compiles in the
 * target's 4 MiB scratch region (1.25 MB at its peak). Run in an online image and in a table image; about a second a
 * program.
 * ------------------------------------------------------------------------- */
static int case_offarmn(void)
{
#define OFFARMN(n) { n, "offarm/fill_color.nodiscard.offarm" #n ".frag", gl_test_offarm##n##_fs_src }
	static const struct offarm_arm arm[] = {
		{ 0, "femtovg/fill_color.nodiscard.frag", gl_test_fvg_fill_color_nodiscard_fs_src },
		OFFARMN(2), OFFARMN(4), OFFARMN(8), OFFARMN(12), OFFARMN(16), OFFARMN(24), OFFARMN(32),
		OFFARMN(48), OFFARMN(64), OFFARMN(96), OFFARMN(128), OFFARMN(192), OFFARMN(256),
	};
#undef OFFARMN

	return offarm_run(arm, (int)(sizeof(arm) / sizeof(arm[0])), "offarmn");
}

/* ---- case 195: an `if` arm predicated or branched past (ifbranch) --------
 *
 * Case 193 measured what a predicated-off group costs: 0.11 to 0.29 ms a
 * frame on the AM62P, rising with N (RESULTS.md, 2026-10-05). The other
 * side of the choice is a branch: pco's lower_if_cond_exec (its
 * `PCO_DEBUG=no_pred_cf`) runs the arms under the execution mask, a `br`
 * taken when no instance is active past each arm, at the price of six
 * control groups (the mask's set-up, `cndst`, two `br`, `cndef`, `cndend`).
 * This times both on one context: 193's four programs (the same sources,
 * key 0x98661, case 39's uniforms, scissorRadius 0, so the then arm --
 * 31, 41, 65, 161 groups -- never runs and the else arm, 23, always does),
 * each compiled with PVR_BACKEND_OPT_IF_LOWERING = PREDICATE and = BRANCH
 * (the option is read at compile and kept by the blend variant the draw
 * builds). Host counts (tests/usc_limits.rs
 * a_predicable_if_branches_as_pcos_no_pred_cf_does):
 *   N           0    8   32  128
 *   predicated  104  114  138  234
 *   branched    110  120  144  240
 * Each program is drawn at scissorRadius 0 (the arm off) and 4 (the arm
 * taken: its steps are the identity), 16 sub-runs in all. Groups issued
 * (the host byte model):
 *   N                        0    8   32  128
 *   predicated, either       104  114  138  234   (every group)
 *   branched, radius 0        79   79   79   79   (the then arm skipped)
 *   branched, radius 4        87   97  121  217   (the else arm's 23 skipped)
 * Logged per program, form and radius: best, 3D busy and TA busy per
 * frame; radius 0 in the line the first run (97ba03d4, 2026-10-06) logged,
 * "ifbranch <i> <form>: off arm ...", radius 4 as "ifbranch <i> <form> on:
 * taken arm ...". At radius 0 every sub-run must draw N = 0's predicated
 * picture, at radius 4 N = 0's predicated radius-4 picture. The first run:
 * the branched rows flat in N (55.34 ms), +0.78 ms at N = 0, so for this
 * program a branch wins past about 7 steps (a never-run arm of about 38
 * groups); radius 4 asks what the branch costs where the arm runs.
 * ------------------------------------------------------------------------- */
static int case_ifbranch(void)
{
	enum { W = 1024, H = 600, OVER = 16, REPS = 5, PROGS = 4, FORMS = 2, ARMS = 2 };
	struct fvg_vertex { float xy[2]; float uv[2]; };
	static struct fvg_vertex quads[OVER * 6];
	static const float xs[6] = { 0, W, W, 0, W, 0 }, ys[6] = { 0, 0, H, 0, H, H };
	static const struct {
		int n;
		const char *file, *fs;
	} prog_src[PROGS] = {
		{ 0, "femtovg/fill_color.nodiscard.frag", gl_test_fvg_fill_color_nodiscard_fs_src },
		{ 8, "offarm/fill_color.nodiscard.offarm8.frag", gl_test_offarm8_fs_src },
		{ 32, "offarm/fill_color.nodiscard.offarm32.frag", gl_test_offarm32_fs_src },
		{ 128, "offarm/fill_color.nodiscard.offarm128.frag", gl_test_offarm128_fs_src },
	};
	static const struct {
		unsigned int v;
		const char *name;
	} form[FORMS] = {
		{ GL_TEST_IF_PREDICATE, "predicate" },
		{ GL_TEST_IF_BRANCH, "branch" },
	};
	float frag[56] = { 0 };
	/* scissorRadius: 0 never runs the arm the steps grow, 4 runs it */
	static const float radius[ARMS] = { 0.0f, 4.0f };
	uint32_t got[PROGS][FORMS][ARMS];
	GLuint tex, fbo, vbo, vs;
	bool ok = true;

	for (int i = 0; i < OVER * 6; i++) {
		quads[i].xy[0] = xs[i % 6];
		quads[i].xy[1] = ys[i % 6];
		quads[i].uv[0] = 0.5f;
		quads[i].uv[1] = 1.0f;
	}
	/* case 39's uniforms (186's at radius 0), and the steps' identity */
	frag[6 * 4 + 0] = 0.3f; frag[6 * 4 + 3] = 0.5f;
	frag[8 * 4 + 0] = 1.0f; frag[8 * 4 + 1] = 1.0f;
	frag[8 * 4 + 2] = 1.0f; frag[8 * 4 + 3] = 1.0f;
	frag[10 * 4 + 0] = 1.0f;
	frag[10 * 4 + 1] = -1.0f;
	frag[10 * 4 + 3] = 5.0f;
	frag[13 * 4 + 1] = 1.0f;
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: ifbranch: FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, W, H);
	glDisable(GL_DEPTH_TEST);
	glEnable(GL_BLEND);
	glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
	vbo = upload_vbo(quads, sizeof(quads));
	vs = compile(GL_VERTEX_SHADER, gl_test_fvg_main_vs_src);
	if (!vs)
		return -1;
	for (int p = 0; p < PROGS; p++) {
		for (int f = 0; f < FORMS; f++) {
			GLuint fs, prog;
			GLint linked = 0;

			/* read when the source compiles; the record keeps it for
			 * the blend variant the first draw compiles */
			gl_test_if_lowering(form[f].v);
			fs = compile(GL_FRAGMENT_SHADER, prog_src[p].fs);
			gl_test_if_lowering(GL_TEST_IF_DEFAULT);
			if (!fs) {
				pr_err("gl_test: ifbranch %d %s: %s does not compile\n", p, form[f].name, prog_src[p].file);
				return -1;
			}
			prog = glCreateProgram();
			glAttachShader(prog, vs);
			glAttachShader(prog, fs);
			glBindAttribLocation(prog, 0, "vertex");
			glBindAttribLocation(prog, 1, "tcoord");
			glLinkProgram(prog);
			glGetProgramiv(prog, GL_LINK_STATUS, &linked);
			if (!linked)
				return -1;
			glUseProgram(prog);
			glUniform2f(glGetUniformLocation(prog, "viewSize"), (float)W, (float)H);
			glBindBuffer(GL_ARRAY_BUFFER, vbo);
			glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)0);
			glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct fvg_vertex), (const void *)8);
			glEnableVertexAttribArray(0);
			glEnableVertexAttribArray(1);
			for (int a = 0; a < ARMS; a++) {
				uint32_t pc0[7], pc1[7], t0, best = 0xffffffffu, busy3d, busyta;
				uint8_t px[4];

				frag[12 * 4 + 3] = radius[a]; /* scissorRadius */
				glUniform4fv(glGetUniformLocation(prog, "frag"), 14, frag);
				for (int r = 0; r < REPS + 1; r++) {
					glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
					glClear(GL_COLOR_BUFFER_BIT);
					if (r == 1)
						gl_test_perf_read(pc0);
					t0 = now_ms();
					glDrawArrays(GL_TRIANGLES, 0, OVER * 6);
					glFinish();
					if (r != 0 && now_ms() - t0 < best)
						best = now_ms() - t0;
				}
				gl_test_perf_read(pc1);
				busy3d = (pc1[1] - pc0[1]) / 800u / REPS;
				busyta = (pc1[0] - pc0[0]) / 800u / REPS;
				/* radius 0: the 97ba03d4 record's line, unchanged */
				if (a == 0)
					pr_info("gl_test: ifbranch %d %s: off arm +%d groups: best %u ms; per frame 3D busy %u us, TA busy %u us\n",
						p, form[f].name, prog_src[p].n, best, busy3d, busyta);
				else
					pr_info("gl_test: ifbranch %d %s on: taken arm +%d groups: best %u ms; per frame 3D busy %u us, TA busy %u us\n",
						p, form[f].name, prog_src[p].n, best, busy3d, busyta);
				glReadPixels(W / 2, H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
				got[p][f][a] = (uint32_t)px[0] | (uint32_t)px[1] << 8 | (uint32_t)px[2] << 16 |
					       (uint32_t)px[3] << 24;
			}
			frag[12 * 4 + 3] = 0.0f;
			glDisableVertexAttribArray(0);
			glDisableVertexAttribArray(1);
			glDeleteProgram(prog);
			glDeleteShader(fs);
		}
	}
	glDisable(GL_BLEND);
	if (!check_gl_error("ifbranch"))
		return -1;
	pr_info("gl_test: ifbranch pixels: %08x %08x %08x %08x %08x %08x %08x %08x\n", got[0][0][0], got[0][1][0],
		got[1][0][0], got[1][1][0], got[2][0][0], got[2][1][0], got[3][0][0], got[3][1][0]);
	pr_info("gl_test: ifbranch pixels on: %08x %08x %08x %08x %08x %08x %08x %08x\n", got[0][0][1], got[0][1][1],
		got[1][0][1], got[1][1][1], got[2][0][1], got[2][1][1], got[3][0][1], got[3][1][1]);
	/* Radius 0: the arm never runs, so every program and form draws N = 0's
	 * predicated picture: innerCol's red 0.3 under the blend, 16 layers.
	 * Radius 4: the arm runs, its steps the identity, so every program and
	 * form draws N = 0's predicated radius-4 picture. */
	for (int a = 0; a < ARMS; a++)
		for (int p = 0; p < PROGS; p++)
			for (int f = 0; f < FORMS; f++)
				if (p != 0 || f != 0)
					ok &= expect_int(a == 0 ? "every program and form draws N = 0's predicated picture"
								: "with the arm taken, every program and form draws N = 0's",
							 got[p][f][a] == got[0][0][a], 1);
	ok &= expect_int("the blend stacks the layers (red past 0x4c)", (got[0][0][0] & 0xff) > 0x4c, 1);
	glDeleteShader(vs);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glDeleteBuffers(1, &vbo);
	return ok ? 0 : -1;
}
/* ---- case 197: dEQP texture.vertex.2d's draw, mipmapped (vtxgrid) ----------
 *
 * dEQP-GLES2.functional.texture.vertex.2d.* (45 cases) came back with the
 * clear colour on the AM62P at 32778b57 (full tree, 2026-10-10, image 9)
 * where they passed at d60b9d76: not a sampling fault but a refused draw.
 * Each case draws dEQP's grid (es2fVertexTextureTests.cpp, GRID_SIZE_2D 127):
 * 127 x 127 quads of four client vertices, a vec2 position and a vec2
 * coordinate, 96,774 client u16 indices -- 1,225,804 bytes of client data
 * per glDrawElements, four draws (viewport quarters) per picture. The
 * PowerVR backend copies a draw's client data into the frame's GENERAL
 * arena, which went from 4 MiB to 1 MiB (dcaafa41), so every such draw was
 * refused ("does not fit an empty frame ... not drawn"). The fix cuts an
 * indexed draw whose client data is over half an arena into pieces of its
 * index list, each copying only the vertices its indices read.
 *
 * This case is that picture with dEQP's shader shape (tools/shaders/gltest/
 * vtxgrid_vs.vert: texture2DLod(u_vtex, a_uv, u_lod) into a varying): the
 * 16x16 GL_RGB chain of case 129 (5 levels, texel (x, y) of level L =
 * (40 + 60 L, 16 x, 16 y)), NEAREST_MIPMAP_NEAREST, CLAMP_TO_EDGE, drawn
 * whole into each 32x32 quarter of the target at lod 0, 1, 2 and 3 (bottom
 * left, bottom right, top left, top right), then read back once. Every pixel
 * must be the texel of level `lod` that its quad's coordinate names (one quad
 * either way allowed, for the pixels on a quad's edge); a refused draw leaves
 * its quarter black, a wrong level shows in blue. One check per quarter, and
 * GL_NO_ERROR. All three GPUs (skipped by name in a table build: no row for
 * its vertex shader); in the set. */
static int case_vtxgrid(void)
{
	static uint8_t lv[16 * 16 * 3];
	static const uint8_t one[3] = { 0x11, 0x22, 0x33 };
	GLuint prog = link_program(gl_test_vtxgrid_vs_src, gl_test_vtl_fs_src), tex[2];
	GLint lp, lu, l_lod;
	bool ok = true;

	if (!prog)
		return -1;
	lp = glGetAttribLocation(prog, "a_pos");
	lu = glGetAttribLocation(prog, "a_uv");
	l_lod = glGetUniformLocation(prog, "u_lod");
	if (lp < 0 || lu < 0 || l_lod < 0) {
		pr_err("vtxgrid: a_pos %d a_uv %d u_lod %d\n", lp, lu, l_lod);
		glDeleteProgram(prog);
		return -1;
	}
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_vtex"), 0);
	glUniform1i(glGetUniformLocation(prog, "u_ftex"), 1);
	glUniform1f(glGetUniformLocation(prog, "u_pick"), 0.0f);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
	glGenTextures(2, tex);
	/* unit 1: the FS's texture (u_pick 0 weighs it out), complete so that
	 * no GPU samples an unbound unit */
	glActiveTexture(GL_TEXTURE1);
	glBindTexture(GL_TEXTURE_2D, tex[1]);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 1, 1, 0, GL_RGB, GL_UNSIGNED_BYTE, one);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, tex[0]);
	for (uint32_t level = 0; level < 5; level++) {
		uint32_t size = 16u >> level;

		for (uint32_t y = 0; y < size; y++)
			for (uint32_t x = 0; x < size; x++) {
				uint32_t t = vtl_texel(level, x, y);

				lv[(y * size + x) * 3] = (uint8_t)t;
				lv[(y * size + x) * 3 + 1] = (uint8_t)(t >> 8);
				lv[(y * size + x) * 3 + 2] = (uint8_t)(t >> 16);
			}
		glTexImage2D(GL_TEXTURE_2D, (GLint)level, GL_RGB, (GLsizei)size, (GLsizei)size, 0, GL_RGB,
			     GL_UNSIGNED_BYTE, lv);
	}
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("vtxgrid upload")) {
		glDeleteTextures(2, tex);
		glDeleteProgram(prog);
		return -1;
	}
	vtl_grid_fill();
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 2, GL_FLOAT, GL_FALSE, 0, vtl_gpos);
	glVertexAttribPointer((GLuint)lu, 2, GL_FLOAT, GL_FALSE, 0, vtl_guv);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lu);
	glViewport(0, 0, TW, TH);
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	/* the four quarters, as dEQP's four cells: lod = quarter index */
	for (uint32_t k = 0; k < 4; k++) {
		glViewport((GLint)((k & 1u) * (TW / 2u)), (GLint)((k >> 1) * (TH / 2u)), (GLsizei)(TW / 2u),
			   (GLsizei)(TH / 2u));
		glUniform1f(l_lod, (float)k);
		glDrawElements(GL_TRIANGLES, (GLsizei)(VTL_GRID * VTL_GRID * 6), GL_UNSIGNED_SHORT, vtl_gidx);
	}
	glFinish();
	glViewport(0, 0, TW, TH);
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lu);
	ok &= gl_test_check(check_gl_error("vtxgrid: four grid draws"), 0, 0);
	read_topdown();
	for (uint32_t k = 0; k < 4; k++) {
		const uint32_t size = 16u >> k, x0 = (k & 1u) * (TW / 2u), y0 = (k >> 1) * (TH / 2u);
		uint32_t bad = 0, fx = 0, fy = 0, fgot = 0, fwant = 0;

		for (uint32_t gy = 0; gy < TH / 2u; gy++)
			for (uint32_t x = 0; x < TW / 2u; x++) {
				/* the quarter's pixel (x, gy) from its bottom, and the quad under it */
				uint32_t got = s_px[(TH - 1u - (y0 + gy)) * TW + x0 + x];
				uint32_t qx = (2u * x + 1u) * VTL_GRID / TW, qy = (2u * gy + 1u) * VTL_GRID / TH;
				uint32_t want = 0;
				bool right = false;

				for (int dy = -1; dy <= 1 && !right; dy++)
					for (int dx = -1; dx <= 1 && !right; dx++) {
						int cx = (int)qx + dx, cy = (int)qy + dy;
						uint32_t tx, ty;

						if (cx < 0 || cy < 0 || cx >= (int)VTL_GRID || cy >= (int)VTL_GRID)
							continue;
						tx = (uint32_t)cx * size / VTL_GRID;
						ty = (uint32_t)cy * size / VTL_GRID;
						want = vtl_texel(k, tx, ty);
						right = chan_diff(got, want) <= 1;
					}
				if (!right && bad++ == 0) {
					fx = x;
					fy = gy;
					fgot = got;
					fwant = vtl_texel(k, qx * size / VTL_GRID, qy * size / VTL_GRID);
				}
			}
		pr_info("gl_test: vtxgrid lod %u (level %u, %ux%u): %u of %u pixels wrong\n", k, k, size, size, bad,
		       (TW / 2u) * (TH / 2u));
		if (bad)
			pr_info("gl_test: vtxgrid lod %u: first wrong (%u, %u) from the quarter's bottom left = 0x%08x, "
			       "want 0x%08x%s\n", k, fx, fy, fgot, fwant,
			       fgot == 0xff000000u ? " (the clear colour: the draw was not made)" : "");
		ok &= gl_test_check(bad == 0, bad, 0);
	}
	glActiveTexture(GL_TEXTURE1);
	glBindTexture(GL_TEXTURE_2D, 0);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(2, tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 196 occl: ES 3.0 occlusion queries (GL_ANY_SAMPLES_PASSED) -------
 *
 * docs/es3-occlusion-query.md; HANDOFF open work 4, occlusion-query. Depth
 * test LESS on a cleared depth buffer, a full-screen red occluder at z -0.5
 * drawn first and outside every query, then five queries:
 *
 *   q0  ANY            a green quad at z 0.5, behind the occluder     -> 0
 *   q1  ANY            a blue quad at z -0.6, in front of it          -> 1
 *   q2  CONSERVATIVE   nothing drawn                                  -> 0
 *   q3  ANY            the hidden quad, glFlush, a cyan quad at z
 *                      -0.7: one query over two kicks                 -> 1
 *   q4  ANY            a yellow quad at z -0.8, visible when drawn;
 *                      after glEndQuery a white quad at z -0.9 covers
 *                      it in the same frame                           -> 1
 *
 * Each counted quad is nearer than everything drawn before it (the test is
 * LESS), so "visible" is exact.
 *
 * q3 is the V3D's counter adding across jobs and the PowerVR's two result
 * areas; q4 is the PowerVR's tile-based hazard (an occluder recorded after
 * glEndQuery must not hide the query's draw: the backend ends the frame at
 * glEndQuery). After glFinish every result must be available. The picture:
 * the white quad in the centre, the red occluder in a corner.
 * ------------------------------------------------------------------------- */
static void occl_quad(struct cuv_vertex *v, float half, float z, float r, float g, float b)
{
	cuv_set(&v[0], -half, -half, z, r, g, b);
	cuv_set(&v[1], half, -half, z, r, g, b);
	cuv_set(&v[2], half, half, z, r, g, b);
	cuv_set(&v[3], -half, -half, z, r, g, b);
	cuv_set(&v[4], half, half, z, r, g, b);
	cuv_set(&v[5], -half, half, z, r, g, b);
}

static int case_occl(void)
{
	enum { OCCLUDER, HIDDEN, VISIBLE, CYAN, YELLOW, WHITE, QUADS };
	static const char *const what[5] = { "q0 ANY hidden", "q1 ANY visible", "q2 CONSERVATIVE empty",
					     "q3 ANY two kicks", "q4 ANY then covered" };
	static const GLuint want[5] = { 0, 1, 0, 1, 1 };
	struct cuv_vertex v[QUADS * 6];
	GLuint prog, vbo, q[5] = { 0 };
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	occl_quad(&v[OCCLUDER * 6], 1.0f, -0.5f, 1, 0, 0);
	occl_quad(&v[HIDDEN * 6], 0.5f, 0.5f, 0, 1, 0);
	occl_quad(&v[VISIBLE * 6], 0.5f, -0.6f, 0, 0, 1);
	occl_quad(&v[CYAN * 6], 0.5f, -0.7f, 0, 1, 1);
	occl_quad(&v[YELLOW * 6], 0.5f, -0.8f, 1, 1, 0);
	occl_quad(&v[WHITE * 6], 0.6f, -0.9f, 1, 1, 1);
	vbo = upload_vbo(v, sizeof(v));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glGenQueries(5, q);
	if (!check_gl_error("occl setup"))
		return -1;

	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, OCCLUDER * 6, 6);
	glBeginQuery(GL_ANY_SAMPLES_PASSED, q[0]);
	glDrawArrays(GL_TRIANGLES, HIDDEN * 6, 6);
	glEndQuery(GL_ANY_SAMPLES_PASSED);
	glBeginQuery(GL_ANY_SAMPLES_PASSED, q[1]);
	glDrawArrays(GL_TRIANGLES, VISIBLE * 6, 6);
	glEndQuery(GL_ANY_SAMPLES_PASSED);
	glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, q[2]);
	glEndQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE);
	glBeginQuery(GL_ANY_SAMPLES_PASSED, q[3]);
	glDrawArrays(GL_TRIANGLES, HIDDEN * 6, 6);
	glFlush();
	glDrawArrays(GL_TRIANGLES, CYAN * 6, 6);
	glEndQuery(GL_ANY_SAMPLES_PASSED);
	glBeginQuery(GL_ANY_SAMPLES_PASSED, q[4]);
	glDrawArrays(GL_TRIANGLES, YELLOW * 6, 6);
	glEndQuery(GL_ANY_SAMPLES_PASSED);
	glDrawArrays(GL_TRIANGLES, WHITE * 6, 6);
	glFinish();
	if (!check_gl_error("occl draws")) {
		ok = false;
		goto out;
	}
	for (int i = 0; i < 5; i++) {
		GLuint avail = 7, res = 7;

		glGetQueryObjectuiv(q[i], GL_QUERY_RESULT_AVAILABLE, &avail);
		glGetQueryObjectuiv(q[i], GL_QUERY_RESULT, &res);
		pr_info("gl_test: occl: %s: available %u result %u (want 1 %u)\n", what[i], avail, res, want[i]);
		ok &= avail == GL_TRUE && res == want[i];
	}
	ok &= check_gl_error("occl results");
	read_topdown();
	ok &= expect_px("occl: the white quad on top", TW / 2, TH / 2, 0xffffffff, 2);
	ok &= expect_px("occl: the occluder in the corner", 2, 2, 0xff0000ff, 2);
out:
	glDisable(GL_DEPTH_TEST);
	glDeleteQueries(5, q);
	glDeleteBuffers(1, &vbo);
	return ok && check_gl_error("occl teardown") ? 0 : -1;
}

/* 176 mapbuf: GL_OES_mapbuffer (glmark2's buffer:update-method=map scenes ask
 * for it). The slice-A triangle is drawn from a VBO; then the whole store is
 * mapped write-only and: nothing written (OES_mapbuffer: "any portion of a
 * mapped buffer that is not written will retain its original contents", so
 * the triangle is still there); every vertex moved to the apex (a degenerate
 * triangle: nothing at the centroid); the triangle written back (there
 * again). GL_BUFFER_ACCESS_OES, GL_BUFFER_MAPPED_OES and the map pointer are
 * checked on the way. */
static int case_mapbuf(void)
{
	struct cuv_vertex tri[3], point[3];
	GLuint prog, vbo;
	GLint iv = -1;
	void *p, *q = NULL;
	bool ok = true;

	ok &= expect_int("GL_OES_mapbuffer advertised",
			 strstr((const char *)glGetString(GL_EXTENSIONS), "GL_OES_mapbuffer") != NULL, 1);
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog || !ok)
		return -1;
	glUseProgram(prog);
	slice_a_vertices(tri, 0);
	point[0] = point[1] = point[2] = tri[0];
	vbo = upload_vbo(tri, sizeof(tri));
	cuv_attribs();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);

	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("mapbuf: before a map"))
		return -1;
	read_topdown();
	ok &= expect_px("mapbuf: the triangle from the VBO", 32, 40, A_CENTROID, 8);

	/* mapped, nothing written: the contents stay */
	glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_ACCESS_OES, &iv);
	ok &= expect_int("BUFFER_ACCESS_OES", iv, GL_WRITE_ONLY_OES);
	p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES);
	ok &= expect_int("glMapBufferOES returns a pointer", p != NULL, 1);
	glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED_OES, &iv);
	ok &= expect_int("BUFFER_MAPPED_OES while mapped", iv, 1);
	glGetBufferPointervOES(GL_ARRAY_BUFFER, GL_BUFFER_MAP_POINTER_OES, &q);
	ok &= expect_int("BUFFER_MAP_POINTER_OES is the mapping", q == p, 1);
	ok &= expect_int("glUnmapBufferOES", glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE);
	glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED_OES, &iv);
	ok &= expect_int("BUFFER_MAPPED_OES after the unmap", iv, 0);
	if (!p || !check_gl_error("mapbuf: an empty map"))
		return -1;
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	read_topdown();
	ok &= expect_px("mapbuf: unwritten, the triangle stays", 32, 40, A_CENTROID, 8);

	/* written: one point, nothing covers the centroid */
	p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES);
	if (!p)
		return -1;
	memcpy(p, point, sizeof(point));
	ok &= expect_int("glUnmapBufferOES 2", glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	read_topdown();
	ok &= expect_px("mapbuf: collapsed, the centroid is empty", 32, 40, 0xff000000, 0);

	/* written back: the triangle again */
	p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES);
	if (!p)
		return -1;
	memcpy(p, tri, sizeof(tri));
	ok &= expect_int("glUnmapBufferOES 3", glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("mapbuf: written back"))
		return -1;
	read_topdown();
	ok &= expect_px("mapbuf: written back, the triangle again", 32, 40, A_CENTROID, 8);

	glDeleteBuffers(1, &vbo);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 169: depth textures, rendered into and sampled (dstex) ----------
 *
 * GL_OES_depth_texture and GL_OES_packed_depth_stencil on an ES 2.0 library,
 * the same tokens in ES 3.0 core. Three steps, each read back:
 *
 * 1. A 4x1 GL_DEPTH_COMPONENT / GL_UNSIGNED_SHORT texture of 0, 1/3, 2/3, 1
 *    drawn over the window, NEAREST: column k reads 85 k in red, and in green
 *    and blue too on ES 2.0 (the extension samples depth as luminance,
 *    (d, d, d, 1)); ES 3.0 treats it as RED, (d, 0, 0, 1).
 * 2. One GL_DEPTH_STENCIL / GL_UNSIGNED_INT_24_8 texture of TW x TH on the
 *    depth AND the stencil point of an FBO with an RGBA colour texture.
 *    Cleared (depth 1, stencil 0); stencil 1 written over the upper-left
 *    triangle (y > x) with no colour; a blue quad whose depth runs from 0 at
 *    the bottom left to 1 at the top right (depth LESS); a green quad at depth
 *    0.5 with stencil EQUAL 1 and depth LESS. Green lands where both pass --
 *    above the main diagonal and beyond the anti-diagonal (the top quarter)
 *    -- and blue everywhere else.
 * 3. That texture sampled over the window: the depth the draws left, 0.5 in
 *    the top quarter and the blue quad's ramp elsewhere. The stencil half is
 *    never sampled (OES_packed_depth_stencil: "as if the base internal
 *    format was DEPTH"). Before 2026-09-30 no backend brought rendered depth
 *    into a texture's texels (texture::resolve_depth), so this read the
 *    upload's contents.
 * ------------------------------------------------------------------------- */
static uint8_t dst_ramp_byte(uint32_t x, uint32_t y)
{
	/* the blue quad's window depth at top-down pixel (x, y): z = (xn + yn) / 2 */
	const float xn = 2.0f * ((float)x + 0.5f) / (float)TW - 1.0f;
	const float yn = 2.0f * ((float)(TH - 1 - y) + 0.5f) / (float)TH - 1.0f;
	const float d = ((xn + yn) * 0.5f + 1.0f) * 0.5f;

	return (uint8_t)(d * 255.0f + 0.5f);
}

/* `tex` over the whole bound framebuffer, through uv.vert / texture.frag. */
static void dst_sample(GLuint prog, GLuint tex)
{
	struct puv_vertex q[6];
	GLint lp = glGetAttribLocation(prog, "a_pos"), lu = glGetAttribLocation(prog, "a_uv");

	puv_quad(q);
	glUseProgram(prog);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_2D, tex);
	glUniform1i(glGetUniformLocation(prog, "u_tex"), 0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), &q[0].pos[0]);
	glVertexAttribPointer((GLuint)lu, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), &q[0].uv[0]);
	glEnableVertexAttribArray((GLuint)lp);
	glEnableVertexAttribArray((GLuint)lu);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray((GLuint)lp);
	glDisableVertexAttribArray((GLuint)lu);
}

static int case_dstex(void)
{
	static const uint16_t ramp[4] = { 0x0000, 0x5555, 0xaaaa, 0xffff };
	/* the upper-left triangle, y > x */
	static const float upper_left[9] = { -1, -1, 0, -1, 1, 0, 1, 1, 0 };
	/* top-down pixels: the top quarter, then the left, bottom and right ones */
	static const uint32_t px[4][2] = { { 32, 4 }, { 4, 32 }, { 32, TH - 5 }, { TW - 5, 32 } };
	static const char *const where[4] = { "top quarter", "left", "bottom", "right" };
	const char *ver = (const char *)glGetString(GL_VERSION);
	const char *ext = (const char *)glGetString(GL_EXTENSIONS);
	const bool es2 = !ver || strncmp(ver, "OpenGL ES 2.", 12) == 0;
	GLuint tprog, dprog, d1 = 0, ds = 0, col = 0, fbo = 0;
	GLint lp;
	bool ok = true;

	if (es2 && (!ext || !strstr(ext, "GL_OES_depth_texture") || !strstr(ext, "GL_OES_packed_depth_stencil"))) {
		pr_info("gl_test: case 169 dstex: SKIP (an ES 2.0 library without GL_OES_depth_texture and "
			"GL_OES_packed_depth_stencil)\n");
		return -3;
	}
	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	tprog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	dprog = link_program(gl_test_dsk_vs_src, gl_test_dsk_fs_src);
	if (!tprog || !dprog)
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_CULL_FACE);

	/* 1. an uploaded depth ramp, sampled */
	glGenTextures(1, &d1);
	glBindTexture(GL_TEXTURE_2D, d1);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT, 4, 1, 0, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT, ramp);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("dstex: the depth ramp's upload"))
		return -1;
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	dst_sample(tprog, d1);
	glFinish();
	ok &= check_gl_error("dstex: sampling the depth ramp");
	read_topdown();
	for (uint32_t k = 0; k < 4; k++) {
		const uint8_t v = (uint8_t)(85 * k);
		char what[40];

		snprintf(what, sizeof(what), "dstex: ramp column %u", (unsigned)k);
		ok &= expect_px(what, 8 + 16 * k, 32, es2 ? rgba_u32(v, v, v, 255) : rgba_u32(v, 0, 0, 255), 2);
	}

	/* 2. depth and stencil rendered into one depth-stencil texture */
	glGenTextures(1, &col);
	glBindTexture(GL_TEXTURE_2D, col);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenTextures(1, &ds);
	glBindTexture(GL_TEXTURE_2D, ds);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_STENCIL_OES, TW, TH, 0, GL_DEPTH_STENCIL_OES,
		     GL_UNSIGNED_INT_24_8_OES, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glBindTexture(GL_TEXTURE_2D, 0);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col, 0);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, ds, 0);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_TEXTURE_2D, ds, 0);
	if (!check_gl_error("dstex: the depth-stencil texture") ||
	    glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: dstex: the framebuffer with a depth-stencil texture is not complete\n");
		ok = false;
		goto out;
	}
	glClearColor(0, 0, 0, 1);
	glClearDepthf(1.0f);
	glClearStencil(0);
	glStencilMask(0xff);
	glDepthMask(GL_TRUE);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	/* stencil 1 over the upper-left triangle, no colour, no depth */
	glEnable(GL_STENCIL_TEST);
	glStencilFunc(GL_ALWAYS, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glUseProgram(dprog);
	lp = glGetAttribLocation(dprog, "a_pos");
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer((GLuint)lp, 3, GL_FLOAT, GL_FALSE, 12, upper_left);
	glEnableVertexAttribArray((GLuint)lp);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glDisableVertexAttribArray((GLuint)lp);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	/* the blue depth ramp */
	glDisable(GL_STENCIL_TEST);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	dsk_quad(dprog, DSK_BLUE, -1.0f, 1.0f);
	/* green at 0.5 where the stencil is 1 and the ramp is farther */
	glEnable(GL_STENCIL_TEST);
	glStencilFunc(GL_EQUAL, 1, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	dsk_quad(dprog, DSK_GREEN, 0.0f, 0.0f);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_DEPTH_TEST);
	glFinish();
	ok &= check_gl_error("dstex: drawing into the depth-stencil texture");
	read_topdown();
	for (int i = 0; i < 4; i++) {
		char what[40];

		snprintf(what, sizeof(what), "dstex: fbo %s", where[i]);
		ok &= expect_px(what, px[i][0], px[i][1], i == 0 ? DSK_GREEN : DSK_BLUE, 1);
	}

	/* 3. the depth those draws left, sampled from the texture */
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glClear(GL_COLOR_BUFFER_BIT);
	dst_sample(tprog, ds);
	glFinish();
	ok &= check_gl_error("dstex: sampling the rendered depth");
	read_topdown();
	for (int i = 0; i < 4; i++) {
		const uint8_t v = i == 0 ? 128 : dst_ramp_byte(px[i][0], px[i][1]);
		char what[40];

		snprintf(what, sizeof(what), "dstex: depth %s", where[i]);
		ok &= expect_px(what, px[i][0], px[i][1], es2 ? rgba_u32(v, v, v, 255) : rgba_u32(v, 0, 0, 255), 2);
	}
out:
	glStencilFunc(GL_ALWAYS, 0, 0xff);
	glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
	glDepthFunc(GL_LESS);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &col);
	glDeleteTextures(1, &ds);
	glDeleteTextures(1, &d1);
	glUseProgram(0);
	glDeleteProgram(tprog);
	glDeleteProgram(dprog);
	return ok ? 0 : -1;
}

/* ---- case 177: a depth texture as the only attachment (zonly) --------------
 *
 * glmark2's shadow scene renders its depth pass into a framebuffer whose only
 * attachment is a GL_DEPTH_COMPONENT / GL_UNSIGNED_INT texture at twice the
 * canvas (scene-shadow.cpp:100-118), and gives up when glCheckFramebufferStatus
 * is not complete. On the i.MX8MP that was its one failed scene (2026-10-04,
 * record 594b120e 47d70027 22781093 03200258). The same steps here, at twice
 * this window:
 * 1. the status: GL_FRAMEBUFFER_COMPLETE, or a SKIP by name where the GPU
 *    renders only with a colour buffer (GL_FRAMEBUFFER_UNSUPPORTED);
 * 2. the depth buffer cleared to 1 and the blue quad's depth ramp drawn into
 *    it, depth test LESS, with no colour buffer to write;
 * 3. the texture sampled over the window: the depth the draw wrote, the ramp
 *    case 169 reads (dst_ramp_byte), at four points.
 * Every step is a numbered check, so a board with no console reads which one
 * failed, and what it read, from the CASE record (check, got, want): 1 the
 * two programs linked, 2 no GL error after the texture and the framebuffer,
 * 3 the status, 4 no GL error after the draw, 5 none after the sampling,
 * 6..9 the four depth points.
 *
 * The shaders are the offline table's (passthrough + uniform colour for the
 * ramp, uv + texture for the sampling), so the case runs in the default
 * gc_gl_test image. Case 169's dsk_* pair is not in the GC7000's offline
 * table ("offline shaderc: vertex shader not in offline table"), which is why
 * the first board run of this case failed at the link; 169 runs with the
 * in-process compiler (prj_sweep.conf).
 * ------------------------------------------------------------------------- */
/* The ramp of dsk_quad(-1, 1) through attribute 0, for the table's
 * passthrough.vert (a_pos at location 0; a_color left disabled). */
static void zonly_ramp(GLuint prog)
{
	const float v[18] = { -1, -1, -1, 1, -1, 0, -1, 1, 0, -1, 1, 0, 1, -1, 0, 1, 1, 1 };

	glUseProgram(prog);
	glUniform4f(glGetUniformLocation(prog, "u_color"), 0.0f, 0.0f, 1.0f, 1.0f);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 12, v);
	glEnableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray(0);
}

static bool expect_u32(const char *what, uint32_t got, uint32_t want); /* below, case 85 */

/* Case 177's steps 1-3 at a depth texture of tw x th, with the two programs
 * already linked (check 1 is the caller's). `name` labels the lines. Sets
 * *status to what glCheckFramebufferStatus answered. */
static bool zonly_steps(const char *name, GLuint tprog, GLuint dprog, GLsizei tw, GLsizei th, bool es2,
			GLenum *status)
{
	static const uint32_t px[4][2] = { { 32, 4 }, { 4, 32 }, { 32, TH - 5 }, { TW - 5, 32 } };
	static const char *const where[4] = { "top", "left", "bottom", "right" };
	GLuint d = 0, fbo = 0;
	bool ok = true;
	char what[48];

	glDisable(GL_STENCIL_TEST);
	glDisable(GL_CULL_FACE);

	/* 1. the shadow pass's target: a depth texture alone */
	glGenTextures(1, &d);
	glBindTexture(GL_TEXTURE_2D, d);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT, tw, th, 0, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, NULL);
	glBindTexture(GL_TEXTURE_2D, 0);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, d, 0);
	*status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	/* check 2: got = the GL error */
	snprintf(what, sizeof(what), "%s: error after setup", name);
	if (!expect_u32(what, glGetError(), GL_NO_ERROR)) {
		pr_info("gl_test:   %s: status 0x%04x\n", name, *status);
		ok = false;
		goto out;
	}
	if (*status == GL_FRAMEBUFFER_UNSUPPORTED) {
		pr_info("gl_test: %s: SKIP (GL_FRAMEBUFFER_UNSUPPORTED: this GPU renders only "
			"with a colour buffer)\n", name);
		goto out;
	}
	/* check 3: got = the status */
	snprintf(what, sizeof(what), "%s: status", name);
	if (!expect_u32(what, *status, GL_FRAMEBUFFER_COMPLETE)) {
		ok = false;
		goto out;
	}

	/* 2. the blue quad's depth ramp, depth only */
	glViewport(0, 0, tw, th);
	glClearDepthf(1.0f);
	glDepthMask(GL_TRUE);
	glClear(GL_DEPTH_BUFFER_BIT);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	zonly_ramp(dprog);
	glDisable(GL_DEPTH_TEST);
	glFinish();
	snprintf(what, sizeof(what), "%s: error after the draw", name);
	ok &= expect_u32(what, glGetError(), GL_NO_ERROR); /* check 4 */

	/* 3. the depth that draw left, sampled over the window */
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	dst_sample(tprog, d);
	glFinish();
	snprintf(what, sizeof(what), "%s: error after sampling", name);
	ok &= expect_u32(what, glGetError(), GL_NO_ERROR); /* check 5 */
	read_topdown();
	for (int i = 0; i < 4; i++) {
		const uint8_t v = dst_ramp_byte(px[i][0], px[i][1]);

		snprintf(what, sizeof(what), "%s: depth %s", name, where[i]);
		ok &= expect_px(what, px[i][0], px[i][1], es2 ? rgba_u32(v, v, v, 255) : rgba_u32(v, 0, 0, 255), 2);
	}
out:
	glDepthFunc(GL_LESS);
	glViewport(0, 0, TW, TH);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &d);
	return ok;
}

/* Whether case 177 / 178 can run here; prints the SKIP line. */
static bool zonly_supported(int no, const char *name, bool *es2)
{
	const char *ver = (const char *)glGetString(GL_VERSION);
	const char *ext = (const char *)glGetString(GL_EXTENSIONS);

	*es2 = !ver || strncmp(ver, "OpenGL ES 2.", 12) == 0;
	if (*es2 && (!ext || !strstr(ext, "GL_OES_depth_texture"))) {
		pr_info("gl_test: case %d %s: SKIP (an ES 2.0 library without GL_OES_depth_texture)\n", no, name);
		return false;
	}
	return true;
}

static int case_zonly(void)
{
	GLuint tprog, dprog;
	GLenum status = 0;
	bool es2, ok;

	if (!zonly_supported(177, "zonly", &es2))
		return -3;
	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	tprog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	dprog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	/* check 1: got = which linked (bit 0 the sampler, bit 1 the ramp) */
	if (!expect_u32("zonly: programs linked", (tprog ? 1u : 0u) | (dprog ? 2u : 0u), 3u))
		return -1;
	/* twice the window, as shadow's texture is twice its canvas */
	ok = zonly_steps("zonly", tprog, dprog, 2 * TW, 2 * TH, es2, &status);
	glUseProgram(0);
	glDeleteProgram(tprog);
	glDeleteProgram(dprog);
	if (ok && status == GL_FRAMEBUFFER_UNSUPPORTED)
		return -3;
	return ok ? 0 : -1;
}

/* ---- case 178: shadow's depth texture right after a scene was deleted (heldfree)
 *
 * glmark2's shadow scene on the AM62P (2026-10-04, measurement round
 * 6242497b): its 1600x1200 GL_DEPTH_COMPONENT texture (7500 KiB) was refused
 * ("render pool exhausted: 7500 KiB requested, 844 KiB left") while 23932 KiB
 * of the previous scenes' frees were held for a kick not yet submitted
 * ("pending frees now 38 (23932 KiB) ... reclaim off"), and the framebuffer
 * read GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT. Case 177's texture is 64 KiB with
 * nothing held, so it cannot see this. Here, with both programs linked first:
 * 1. the GPU pool filled with 1 MiB buffers until glBufferData answers
 *    GL_OUT_OF_MEMORY (at most HF_MAX_MIB), then one glFinish, then every one
 *    deleted with no frame open: the previous scene's teardown;
 * 2. at once, case 177's steps at shadow's own size, 1600x1200.
 * Check 1: the programs linked; check 2: the fill reached at least 8 MiB
 * (got = the MiB it allocated); then case 177's checks 2-9 (check 3..10
 * here). With the OSAL holding a free made with no frame open for the kick
 * after the last submitted one (CONFIG_YAKOGL_PVR_POOL_RECLAIM off) the
 * texture is refused and check 3 reads GL_OUT_OF_MEMORY (0x505); with the
 * reclaim the frees come back and every check passes. GPU-generic; it
 * skips where 177 does.
 * ------------------------------------------------------------------------- */
#define HF_MAX_MIB 96
#define HF_MIN_MIB 8

static int case_heldfree(void)
{
	static GLuint bufs[HF_MAX_MIB];
	GLuint tprog, dprog;
	GLenum status = 0;
	unsigned n = 0;
	bool es2, ok;

	if (!zonly_supported(178, "heldfree", &es2))
		return -3;
	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	tprog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	dprog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	if (!expect_u32("heldfree: programs linked", (tprog ? 1u : 0u) | (dprog ? 2u : 0u), 3u))
		return -1;

	/* 1. the previous scene: the pool filled, then deleted between frames */
	glGenBuffers(HF_MAX_MIB, bufs);
	for (n = 0; n < HF_MAX_MIB; n++) {
		glBindBuffer(GL_ARRAY_BUFFER, bufs[n]);
		glBufferData(GL_ARRAY_BUFFER, 1024 * 1024, NULL, GL_STATIC_DRAW);
		if (glGetError() != GL_NO_ERROR)
			break;
	}
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glFinish();
	glDeleteBuffers(HF_MAX_MIB, bufs);
	while (glGetError() != GL_NO_ERROR)
		;
	pr_info("gl_test:   heldfree: %u MiB allocated, then deleted with no frame open\n", n);
	/* check 2: got = the MiB the fill allocated */
	ok = expect_u32("heldfree: filled MiB >= 8", n >= HF_MIN_MIB ? HF_MIN_MIB : n, HF_MIN_MIB);

	/* 2. shadow's set-up and depth pass at its own size */
	if (ok)
		ok = zonly_steps("heldfree", tprog, dprog, 1600, 1200, es2, &status);
	glUseProgram(0);
	glDeleteProgram(tprog);
	glDeleteProgram(dprog);
	if (ok && status == GL_FRAMEBUFFER_UNSUPPORTED)
		return -3;
	return ok ? 0 : -1;
}

/* ---- case 179: a depth texture re-tiled every frame (retile) ---------------
 *
 * glmark2's shadow renders its depth texture and samples it every frame. On
 * the V3D each new depth is read back into the texture's R32F storage (a new
 * stamp) and tiled again for the TMU; the copy of the old stamp is given
 * back before the sampling job opens (src/v3d/backend.rs
 * drop_stale_copies), so the OSAL does not hold it for that job while the
 * new copy is made: the old and the new copy were both in memory, two
 * 7.8 MB copies for shadow's 1600x1200 map. Two frames, each:
 * 1. a 256x256 depth texture alone in an FBO (as case 177), cleared to 1,
 *    a full quad at window depth z drawn with depth test LESS (frame 1
 *    z = 0.25, frame 2 z = 0.75), no colour buffer;
 * 2. the texture sampled over the window; on the V3D the carve-out's use is
 *    read right after that draw, while its job is still open;
 * 3. the window read back: the frame's depth at the centre and a corner,
 *    (v, v, v, 1) on ES 2.0 and (v, 0, 0, 1) on ES 3.0, v 0x40 then 0xbf
 *    (frame 2's value also proves its texture was tiled again: a stale copy
 *    reads 0x40).
 * Checks: 1 the two programs linked, 2 the status COMPLETE, 3-4 frame 1's
 * two points, 5-6 frame 2's, 7 no GL error; on the V3D 8: the carve-out in
 * use after frame 2's sampling draw minus after frame 1's is 0 (the old copy
 * held while the new one is made reads +256 KiB and more). SKIP by name
 * where the framebuffer is GL_FRAMEBUFFER_UNSUPPORTED or an ES 2.0 library
 * has no GL_OES_depth_texture.
 * ------------------------------------------------------------------------- */
static void retile_quad(GLuint prog, float z)
{
	const float v[18] = { -1, -1, z, 1, -1, z, -1, 1, z, -1, 1, z, 1, -1, z, 1, 1, z };

	glUseProgram(prog);
	glUniform4f(glGetUniformLocation(prog, "u_color"), 0.0f, 0.0f, 1.0f, 1.0f);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 12, v);
	glEnableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray(0);
}

static int case_retile(void)
{
	static const float ndc_z[2] = { -0.5f, 0.5f }; /* window depth 0.25, 0.75 */
	static const uint8_t want_v[2] = { 0x40, 0xbf };
	static const uint32_t px[2][2] = { { 32, 32 }, { 4, 4 } };
	const char *ver = (const char *)glGetString(GL_VERSION);
	const char *ext = (const char *)glGetString(GL_EXTENSIONS);
	const bool es2 = !ver || strncmp(ver, "OpenGL ES 2.", 12) == 0;
	GLuint tprog, dprog, d = 0, fbo = 0;
	GLenum status = 0;
	bool ok = true;
#ifdef GL_TEST_V3D
	uint32_t used[2] = { 0, 0 };
#endif

	if (es2 && (!ext || !strstr(ext, "GL_OES_depth_texture"))) {
		pr_info("gl_test: case 179 retile: SKIP (an ES 2.0 library without GL_OES_depth_texture)\n");
		return -3;
	}
	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	tprog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	dprog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	/* check 1: got = which linked (bit 0 the sampler, bit 1 the quad) */
	if (!expect_u32("retile: programs linked", (tprog ? 1u : 0u) | (dprog ? 2u : 0u), 3u))
		return -1;
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_CULL_FACE);
	glGenTextures(1, &d);
	glBindTexture(GL_TEXTURE_2D, d);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT, 256, 256, 0, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT, NULL);
	glBindTexture(GL_TEXTURE_2D, 0);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, d, 0);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	if (status == GL_FRAMEBUFFER_UNSUPPORTED) {
		pr_info("gl_test: case 179 retile: SKIP (GL_FRAMEBUFFER_UNSUPPORTED: this GPU renders only "
			"with a colour buffer)\n");
		goto out;
	}
	/* check 2: got = the status */
	if (!expect_u32("retile: status", status, GL_FRAMEBUFFER_COMPLETE)) {
		ok = false;
		goto out;
	}
	for (int f = 0; f < 2; f++) {
		/* 1. the depth pass */
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glViewport(0, 0, 256, 256);
		glClearDepthf(1.0f);
		glDepthMask(GL_TRUE);
		glClear(GL_DEPTH_BUFFER_BIT);
		glEnable(GL_DEPTH_TEST);
		glDepthFunc(GL_LESS);
		retile_quad(dprog, ndc_z[f]);
		glDisable(GL_DEPTH_TEST);
		/* 2. the texture sampled over the window, no glFinish between:
		 * the sampling resolves the depth itself */
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glViewport(0, 0, TW, TH);
		glClearColor(0, 0, 0, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		dst_sample(tprog, d);
#ifdef GL_TEST_V3D
		/* the sampling draw's job is still open here */
		osal_v3d_mem_used(&used[f], NULL, NULL);
#endif
		/* 3. read back (this ends the job) */
		read_topdown();
		for (int i = 0; i < 2; i++) {
			const uint8_t v = want_v[f];
			char what[48];

			snprintf(what, sizeof(what), "retile: frame %d (%u,%u)", f + 1, px[i][0], px[i][1]);
			ok &= expect_px(what, px[i][0], px[i][1], es2 ? rgba_u32(v, v, v, 255) : rgba_u32(v, 0, 0, 255),
					2); /* checks 3-6 */
		}
	}
	ok &= expect_u32("retile: GL error", glGetError(), GL_NO_ERROR); /* check 7 */
#ifdef GL_TEST_V3D
	/* check 8: frame 2's carve-out use against frame 1's, as a signed
	 * difference (0 wanted; the old copy held reads 256 KiB and more) */
	pr_info("gl_test:   retile: carve-out used after the sampling draw: frame 1 %u, frame 2 %u bytes\n", used[0],
		used[1]);
	ok &= expect_u32("retile: carve-out frame 2 - frame 1", used[1] - used[0], 0u);
#endif
out:
	glDepthFunc(GL_LESS);
	glViewport(0, 0, TW, TH);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &d);
	glUseProgram(0);
	glDeleteProgram(tprog);
	glDeleteProgram(dprog);
	if (ok && status == GL_FRAMEBUFFER_UNSUPPORTED)
		return -3;
	return ok ? 0 : -1;
}

/* ---- case 180: memory a finished job held, back at once (teardown) --------
 *
 * glmark2 tears a scene down and sets the next one up with no wait in
 * between. On the V3D the OSAL learns that a job completed only when it
 * looks (FRDONE has no interrupt), so the torn-down scene's frees stayed
 * held for a job the GPU had long finished, the next setup allocated around
 * them, and refract never found 7.7 MB in one piece (tools/cts/RESULTS.md,
 * the memory lines). The OSAL now looks without blocking before it fences a
 * free and before it places an allocation while frees are held
 * (v3d_frees_poll). The case:
 * 1. texture A, 1024x1024 RGBA8 red, sampled over the window and read back;
 * 2. A sampled again and glFlush (the job submitted, not waited for); on
 *    the V3D a 20 ms sleep, so the GPU finishes with nobody looking, then
 *    the carve-out in use is read;
 * 3. A deleted, texture B of the same size made green, and on the V3D the
 *    carve-out read again;
 * 4. B sampled over the window and read back.
 * Checks: 1 the program linked, 2 A red at the centre, 3 B green at the
 * centre, 4 no GL error; on the V3D 5: the carve-out after B's creation has
 * not grown past what it was before A's deletion (A's storage and tiled copy
 * held while B's are made reads +4 MiB and more). All three GPUs.
 * ------------------------------------------------------------------------- */
static GLuint teardown_tex(uint8_t r, uint8_t g, uint8_t b)
{
	static uint8_t row[1024 * 4];
	GLuint t = 0;

	fill_texels(row, 1024, r, g, b, 255);
	glGenTextures(1, &t);
	glBindTexture(GL_TEXTURE_2D, t);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1024, 1024, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	for (int y = 0; y < 1024; y++)
		glTexSubImage2D(GL_TEXTURE_2D, 0, 0, y, 1024, 1, GL_RGBA, GL_UNSIGNED_BYTE, row);
	glBindTexture(GL_TEXTURE_2D, 0);
	return t;
}

static int case_teardown(void)
{
	GLuint prog, a, b;
	bool ok = true;
#ifdef GL_TEST_V3D
	uint32_t before = 0, after = 0;
#endif

	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!expect_u32("teardown: program linked", prog ? 1u : 0u, 1u)) /* check 1 */
		return -1;
	glViewport(0, 0, TW, TH);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_CULL_FACE);
	glDisable(GL_BLEND);
	/* 1. A, sampled and read back */
	a = teardown_tex(255, 0, 0);
	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	dst_sample(prog, a);
	read_topdown();
	ok &= expect_px("teardown: A", 32, 32, rgba_u32(255, 0, 0, 255), 2); /* check 2 */
	/* 2. A sampled again, the job submitted and not waited for */
	glClear(GL_COLOR_BUFFER_BIT);
	dst_sample(prog, a);
	glFlush();
#ifdef GL_TEST_V3D
	k_msleep(20); /* the GPU finishes; the OSAL is not asked */
	osal_v3d_mem_used(&before, NULL, NULL);
#endif
	/* 3. A deleted, B of the same size made */
	glDeleteTextures(1, &a);
	b = teardown_tex(0, 255, 0);
#ifdef GL_TEST_V3D
	osal_v3d_mem_used(&after, NULL, NULL);
#endif
	/* 4. B sampled and read back */
	glClear(GL_COLOR_BUFFER_BIT);
	dst_sample(prog, b);
	read_topdown();
	ok &= expect_px("teardown: B", 32, 32, rgba_u32(0, 255, 0, 255), 2); /* check 3 */
	ok &= expect_u32("teardown: GL error", glGetError(), GL_NO_ERROR);  /* check 4 */
#ifdef GL_TEST_V3D
	pr_info("gl_test:   teardown: carve-out used before A's deletion %u, after B's creation %u bytes\n", before,
		after);
	/* check 5: got = how much it grew, 0 when it did not */
	ok &= expect_u32("teardown: carve-out growth", after > before ? after - before : 0u, 0u);
#endif
	glBindTexture(GL_TEXTURE_2D, 0);
	glDeleteTextures(1, &b);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
}

/* ---- case 148: look-ups inside uniform branches ----------------------------
 *
 * es100/texbranch.frag, with ta = (204, 153, 0, 255) on unit 0 and
 * tb = (51, 0, 255, 255) on unit 1, both one texel, NEAREST:
 *   red   `if (u.x > 0.5)` ta's red, else tb's red;
 *   green `if (u.y > 0.5)` ta's green, else untouched (0);
 *   blue  for i = 1..24, break when i >= u.z, two look-ups of tb's blue a
 *         pass, each 1/48: (ceil(u.z) - 1) * 2 / 48 of tb's blue.
 * Three draws: u = (1, 1, 4, 0): (204, 153, 32); (0, 0, 1, 0): (51, 0, 0),
 * where the one-armed `if` and every pass are skipped; (0, 1, 25, 0):
 * (51, 153, 255), all 24 passes. Every channel differs between two of the
 * draws by far more than the tolerance, so a wrong arm, a lost merge, a
 * look-up result from the untaken arm or a pass past the break shows. On
 * the V3D each `if` and the break guard are uniform branches, and the last
 * thread switch comes after them (select.rs last_thrsw).
 * ------------------------------------------------------------------------- */
static GLuint texbranch_texel(GLenum unit, const uint8_t rgba[4])
{
	GLuint tex = 0;

	glActiveTexture(unit);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, rgba);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	return tex;
}

static int case_texbranch(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const struct { int x, y; } at[3] = { { 8, 8 }, { 40, 20 }, { 55, 57 } };
	static const uint8_t ta[4] = { 204, 153, 0, 255 }, tb[4] = { 51, 0, 255, 255 };
	static const struct {
		float u[4];
		uint32_t want;
		const char *what;
	} draw[3] = {
		{ { 1.0f, 1.0f, 4.0f, 0.0f }, 0xff2099ccu, "then arm, gate on, 3 passes" },
		{ { 0.0f, 0.0f, 1.0f, 0.0f }, 0xff000033u, "else arm, gate off, no pass" },
		{ { 0.0f, 1.0f, 25.0f, 0.0f }, 0xffff9933u, "else arm, gate on, 24 passes" },
	};
	GLuint prog, texa, texb;
	GLint loc;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_deriv_vs_src, gl_test_texbranch_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	texa = texbranch_texel(GL_TEXTURE0, ta);
	texb = texbranch_texel(GL_TEXTURE1, tb);
	glUniform1i(glGetUniformLocation(prog, "ta"), 0);
	glUniform1i(glGetUniformLocation(prog, "tb"), 1);
	loc = glGetUniformLocation(prog, "u");
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);
	for (int d = 0; d < 3; d++) {
		glUniform4fv(loc, 1, draw[d].u);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(draw[d].what)) {
			ok = false;
			break;
		}
		read_topdown();
		for (int p = 0; p < 3; p++) {
			char what[80];

			snprintf(what, sizeof(what), "texbranch: %s", draw[d].what);
			ok &= expect_px(what, at[p].x, at[p].y, draw[d].want, 2);
		}
	}
	glActiveTexture(GL_TEXTURE0);
	glDeleteTextures(1, &texa);
	glDeleteTextures(1, &texb);
	return ok ? 0 : -1;
}

/* ---- case 142: a uniform branch, both arms read back -----------------------
 *
 * es100/unibranch.frag: `if (u.x > 0.5) { then } else { else }` where both
 * arms compute a colour from u.yzw with more than 24 scalar operations, so
 * the V3D back end branches round the untaken arm (bu.anynap to the else
 * arm, bu over it) instead of computing both and selecting. Drawn with
 * u.x = 1 and u.x = 0, u.yzw = (0.9, 0.3, 0.6); each arm's colour is
 * computed here with the same formula, and the two differ in every channel
 * by more than the tolerance, so the wrong arm, a lost merge or a uniform
 * read from the skipped arm (the stream not moved past it) shows.
 * ------------------------------------------------------------------------- */
static void unibranch_colour(bool then_arm, float y, float z, float w, float out[4])
{
	float a[4], b[4], c[4];

	if (then_arm) {
		a[0] = y; a[1] = z; a[2] = w; a[3] = 0.5f;
		for (int i = 0; i < 4; i++)
			b[i] = a[i] * a[(i + 1) % 4] + a[(i + 2) % 4] * a[(i + 3) % 4];
		for (int i = 0; i < 4; i++)
			c[i] = b[i] * 0.5f - a[i] * 0.25f + a[3 - i] * 0.125f;
		for (int i = 0; i < 4; i++) {
			float x = c[i] * c[(i + 1) % 4] + a[i] * 0.5f;
			out[i] = x < 0.0f ? 0.0f : x > 1.0f ? 1.0f : x;
		}
	} else {
		a[0] = w; a[1] = z; a[2] = y; a[3] = 0.25f;
		for (int i = 0; i < 4; i++)
			b[i] = a[i] * a[i] + a[(i + 1) % 4] * 0.5f;
		for (int i = 0; i < 4; i++)
			c[i] = b[i] * a[(i + 2) % 4] - a[i] * 0.125f + a[(i + 3) % 4] * 0.25f;
		for (int i = 0; i < 4; i++) {
			float x = c[i] + c[(i + 1) % 4] * 0.25f;
			out[i] = x < 0.0f ? 0.0f : x > 1.0f ? 1.0f : x;
		}
	}
}

static uint32_t unibranch_want(bool then_arm)
{
	float c[4];
	uint32_t px = 0xff000000u;

	unibranch_colour(then_arm, 0.9f, 0.3f, 0.6f, c);
	for (int i = 0; i < 3; i++)
		px |= (uint32_t)(c[i] * 255.0f + 0.5f) << (8 * i);
	return px;
}

static int case_unibranch(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const struct { int x, y; } at[3] = { { 8, 8 }, { 40, 20 }, { 55, 57 } };
	const uint32_t want[2] = { unibranch_want(false), unibranch_want(true) };
	GLuint prog;
	GLint loc;
	bool ok = true;

	/* the two arms must be told apart: every channel further apart than
	 * the tolerance */
	for (int i = 0; i < 3; i++) {
		int d = (int)((want[0] >> (8 * i)) & 0xff) - (int)((want[1] >> (8 * i)) & 0xff);

		if (d >= -2 && d <= 2) {
			pr_info("gl_test:   unibranch: the arms' colours 0x%08x and 0x%08x are too close\n",
				want[1], want[0]);
			return -1;
		}
	}
	free_all_objects();
	prog = link_program(gl_test_deriv_vs_src, gl_test_unibranch_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "u");
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);
	for (int then_arm = 1; then_arm >= 0; then_arm--) {
		glUniform4f(loc, then_arm ? 1.0f : 0.0f, 0.9f, 0.3f, 0.6f);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(then_arm ? "unibranch: then arm" : "unibranch: else arm"))
			return -1;
		read_topdown();
		for (int p = 0; p < 3; p++) {
			char what[64];

			snprintf(what, sizeof(what), "unibranch: %s arm", then_arm ? "then" : "else");
			ok &= expect_px(what, at[p].x, at[p].y, want[then_arm], 2);
		}
	}
	return ok ? 0 : -1;
}

/* ---- case 138: dFdx / dFdy on the default framebuffer and on an FBO -------
 *
 *     gl_FragColor = (0.5 + 8 dFdx(v.x), 0.5 + 8 dFdy(v.y),
 *                     0.5 + (dFdx(gl_FragCoord.x) + dFdy(gl_FragCoord.y)) / 8, 1)
 *
 * es100/deriv.*: v is the quad's NDC position, so on the 64x64 target every
 * derivative is 1/32 (v) or 1 (gl_FragCoord) with GL's y running up, and red,
 * green and blue read 0.75 = 191 (0xffbfbfbf) everywhere, on both targets. A
 * dFdy taken along the target's rows without the window flip reads green 0.25
 * = 64 and blue 0.5 = 128 on exactly one of the two targets (the default
 * framebuffer is stored top-down, an FBO is not); no derivative reads 128 in
 * red and green. Each GPU's derivatives (the V3D's FDX/FDY, the GC7000's
 * dsx/dsy, the PowerVR's) are measured only here: the host interpreters do
 * not model a quad's lanes. Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_deriv(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const struct { int x, y; } at[3] = { { 8, 8 }, { 40, 20 }, { 55, 57 } };
	GLuint prog, tex = 0, fbo = 0;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_deriv_vs_src, gl_test_deriv_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);

	for (int target = 0; target < 2; target++) {
		if (target == 1) {
			glGenTextures(1, &tex);
			glBindTexture(GL_TEXTURE_2D, tex);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
			glGenFramebuffers(1, &fbo);
			glBindFramebuffer(GL_FRAMEBUFFER, fbo);
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
		}
		glViewport(0, 0, TW, TH);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error(target ? "deriv: fbo" : "deriv: default framebuffer"))
			return -1;
		read_topdown();
		for (int p = 0; p < 3; p++) {
			char what[96];

			snprintf(what, sizeof(what), "deriv: %s (%d,%d)", target ? "fbo" : "default framebuffer",
				 at[p].x, at[p].y);
			ok &= expect_px(what, at[p].x, at[p].y, 0xffbfbfbfu, 2);
		}
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 82: gl_FragCoord, in the default framebuffer and in an FBO -------
 * es100/fragcentre.frag writes R = 4 floor(x), G = 4 floor(y), B = 128 fract(x)
 * + 64 fract(y). GL's window coordinates count y from the bottom in both
 * targets, and a pixel's centre is at +0.5 (B = 96). The default framebuffer
 * is stored top-down (flip_y) and FBOs are not, so a shader that reads the
 * hardware's pixel position without the flip gets G = 4 (63 - y) there and the
 * right answer in the FBO: the per-draw (scale, offset) the record will carry
 * exists for exactly that. Readback is raw glReadPixels, GL's rows (row 0 =
 * bottom), so a check at GL (x, y) names the same pixel in both targets. */
static bool expect_rgb_tol(const char *what, uint32_t got, uint32_t want, int tol)
{
	bool ok = chan_diff(got | 0xff000000u, want | 0xff000000u) <= tol;

	pr_info("gl_test:   %-28s = 0x%08x  want 0x%08x  %s  [check %d]\n",
		what, got, want, ok ? "ok" : "MISMATCH", gl_test_check_no + 1);
	return gl_test_check(ok, got, want);
}

static uint32_t s_gl_rows[TW * TH];

static bool fragcentre_checks(const char *target)
{
	static const uint32_t at[4][2] = { { 0, 0 }, { 63, 0 }, { 0, 63 }, { 40, 20 } };
	bool ok = true;

	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_gl_rows);
	for (int i = 0; i < 4; i++) {
		uint32_t x = at[i][0], y = at[i][1];
		char what[GL_TEST_LABEL];

		snprintf(what, sizeof(what), "%s GL (%u,%u)", target, x, y);
		ok &= expect_rgb_tol(what, s_gl_rows[y * TW + x],
				     rgba_u32((uint8_t)(4 * x), (uint8_t)(4 * y), 96, 255), 1);
	}
	return ok;
}

static void fullscreen_pc_quad(float r, float g, float b)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	static struct pc_vertex v[6];

	for (int i = 0; i < 6; i++) {
		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = r; v[i].color[1] = g; v[i].color[2] = b; v[i].color[3] = 1.0f;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
}

static int case_fragcentre(void)
{
	GLuint prog, tex = 0, fbo = 0;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_fragcentre_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	fullscreen_pc_quad(1.0f, 1.0f, 1.0f);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("fragcentre default"))
		return -1;
	ok &= fragcentre_checks("default fb");

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: fragcentre FBO incomplete\n");
		return -1;
	}
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("fragcentre fbo"))
		return -1;
	ok &= fragcentre_checks("fbo");
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok ? 0 : -1;
}

/* ---- case 119: gl_FragCoord and winding in an EGL pbuffer ------------------
 * Case 82's shader and checks, drawn into a pbuffer instead of the harness's
 * own surface. A pbuffer is never scanned out, so it is stored y-up as an FBO
 * is (GlContext::default_y_up, fb_flip_y false): the hardware's pixel position
 * is GL's, and a shader that reads it unmirrored (pco's table programs, which
 * carry no (scale, offset) pair) gets G = 4 y. Stored top-down, as every
 * default surface was before, that shader gets G = 4 (63 - y) here -- dEQP's
 * fragcoord_xyz, which runs on pbuffers. Back-face culling is on and the quad
 * is counter-clockwise, so a winding that did not follow the flip draws
 * nothing and every check reads the clear. Readback is glReadPixels (GL rows),
 * so the checks do not depend on how the surface is stored. */
static int case_pbufup(void)
{
	/* 8 bits a channel: with no colour size asked for, EGL 1.5 3.4.1.2 sorts
	 * the smaller EGL_BUFFER_SIZE first and the checks would read RGB565
	 * (board, 2a57943d: every channel off by exactly 5/6-bit quantisation,
	 * G = 4 y and unmirrored) */
	static const EGLint cfg_attribs[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RED_SIZE, 8,
					      EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_NONE };
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLSurface pb;
	EGLContext ctx;
	EGLConfig cfg;
	EGLint n = 0;
	GLuint prog;
	bool ok = true;

	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1)
		return -1;
	pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (pb == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, pb, pb, ctx)) {
		pr_err("gl_test: pbufup: surface/context failed (0x%x)\n", eglGetError());
		return -1;
	}
	prog = link_program(gl_test_passthrough_vs_src, gl_test_fragcentre_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	fullscreen_pc_quad(1.0f, 1.0f, 1.0f);
	glViewport(0, 0, TW, TH);
	glEnable(GL_CULL_FACE);
	glCullFace(GL_BACK);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("pbufup draw"))
		return -1;
	ok &= fragcentre_checks("pbuffer");
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, pb);
	eglDestroyContext(dpy, ctx);
	eglTerminate(dpy);
	return ok ? 0 : -1;
}

/* ---- case 83: gl_PointCoord across a 16-pixel point sprite -----------------
 * points.vert (a per-vertex gl_PointSize) with es100/pointcoord.frag
 * (gl_FragColor = (s, t, 0, 1)). One 16-pixel point centred on the pixel
 * corner (32, 32) covers GL pixels 24..39 in x and y; ES 2.0 3.3 gives
 * s = 1/2 + (x + 1/2 - 32) / 16 and t = 1/2 - (y + 1/2 - 32) / 16, so t runs
 * from the TOP of the sprite. The vertex colour is (0.25, 0.75, 0.5) and the
 * fragment shader does not read it: a point coordinate taken from the wrong
 * coefficients shows up as that colour (0x40, 0xbf) instead of a ramp. */
static int case_pointcoord(void)
{
	static const uint32_t at[4][2] = { { 24, 39 }, { 39, 39 }, { 24, 24 }, { 39, 24 } };
	struct pv { float pos[4]; float color[4]; float size; } pv = {
		{ 0.0f, 0.0f, 0.0f, 1.0f }, { 0.25f, 0.75f, 0.5f, 1.0f }, 16.0f,
	};
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_points_vs_src, gl_test_pointcoord_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv.pos);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv.color);
	glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, sizeof(struct pv), &pv.size);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_POINTS, 0, 1);
	glFinish();
	glDisableVertexAttribArray(2);
	if (!check_gl_error("pointcoord"))
		return -1;
	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_gl_rows);
	for (int i = 0; i < 4; i++) {
		uint32_t x = at[i][0], y = at[i][1];
		float sc = 0.5f + ((float)x + 0.5f - 32.0f) / 16.0f;
		float tc = 0.5f - ((float)y + 0.5f - 32.0f) / 16.0f;
		char what[GL_TEST_LABEL];

		snprintf(what, sizeof(what), "sprite GL (%u,%u) s,t", x, y);
		ok &= expect_rgb_tol(what, s_gl_rows[y * TW + x],
				     rgba_u32((uint8_t)(sc * 255.0f + 0.5f), (uint8_t)(tc * 255.0f + 0.5f), 0, 255), 2);
	}
	ok &= expect_rgb_tol("outside the sprite GL (23,32)", s_gl_rows[32 * TW + 23], rgba_u32(0, 0, 0, 255), 0);
	return ok ? 0 : -1;
}

/* ---- case 105: gl_PointCoord after a varying ---------------------------------
 * Case 83's sprite with es100/pointcoordvary.frag: (s, t, v_color.b, 1). The
 * varying is the first fragment input, so gl_PointCoord's components start at
 * 4 -- on the GC7000 that is SH_SPECIALS' PS_PCOORD_IN = 4 and components 4
 * and 5 marked POINTCOORD_X/Y (Mesa's etnaviv) -- an offset case 83, whose
 * shader reads nothing else, cannot tell apart from none. Blue is the
 * vertex colour's 0.5: the varying still arrives whole beside it.
 * Predictions (GC7000, online): the four corner ramps within 2, blue 0x80
 * within 2, black outside. A point coordinate read at offset 0 would give
 * R, G = the colour's 0x40, 0xbf. */
static int case_pointcoordvary(void)
{
	static const uint32_t at[4][2] = { { 24, 39 }, { 39, 39 }, { 24, 24 }, { 39, 24 } };
	struct pv { float pos[4]; float color[4]; float size; } pv = {
		{ 0.0f, 0.0f, 0.0f, 1.0f }, { 0.25f, 0.75f, 0.5f, 1.0f }, 16.0f,
	};
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_points_vs_src, gl_test_pointcoordvary_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv.pos);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pv), pv.color);
	glVertexAttribPointer(2, 1, GL_FLOAT, GL_FALSE, sizeof(struct pv), &pv.size);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_POINTS, 0, 1);
	glFinish();
	glDisableVertexAttribArray(2);
	if (!check_gl_error("pointcoordvary"))
		return -1;
	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, s_gl_rows);
	for (int i = 0; i < 4; i++) {
		uint32_t x = at[i][0], y = at[i][1];
		float sc = 0.5f + ((float)x + 0.5f - 32.0f) / 16.0f;
		float tc = 0.5f - ((float)y + 0.5f - 32.0f) / 16.0f;
		char what[GL_TEST_LABEL];

		snprintf(what, sizeof(what), "sprite GL (%u,%u) s,t,b", x, y);
		ok &= expect_rgb_tol(what, s_gl_rows[y * TW + x],
				     rgba_u32((uint8_t)(sc * 255.0f + 0.5f), (uint8_t)(tc * 255.0f + 0.5f), 0x80, 255), 2);
	}
	ok &= expect_rgb_tol("outside the sprite GL (23,32)", s_gl_rows[32 * TW + 23], rgba_u32(0, 0, 0, 255), 0);
	return ok ? 0 : -1;
}

/* ---- case 85: the 1x1 faces of a cube's last level, rendered and read back --
 * A cube map is stored with its six faces stacked; fde0086 lays a 1-wide
 * level out one texel a row (the TPU reads it at its own width), so the six
 * 1x1 faces of a 4x4 cube's level 2 sit ONE texel apart where they used to
 * sit a pair apart. The PBE writes a render target's rows at a pair pitch.
 * If it writes the pair's second pixel too, rendering face f puts its
 * padding on face f+1's texel. Qt Quick 3D's IBL prefilter renders every
 * level of its light probe face by face, so this is the one path fde0086
 * could break without failing a dEQP case.
 *
 * Each face of level 2 is attached and cleared to its own colour (the clear
 * is a PBE write), in face order 0..5 and then, with other colours, in
 * reverse order 5..0; after each pass all six are read back through the FBO.
 * In face order a pair-writing PBE is repaired by the next face's own write
 * (only face 5's padding lands past the level, in the rounded-row slack), so
 * that pass alone cannot see it; the reverse pass leaves face f+1's texel
 * holding face f's colour for f = 0..4.
 *
 * Not included: sampling level 2. GL_TEXTURE_BASE_LEVEL is accepted and not
 * modelled, and a shader with an explicit LOD above 0 would be a new table
 * entry (a regenerated table needs its own board runs). */
static bool expect_u32(const char *what, uint32_t got, uint32_t want)
{
	bool ok = got == want;

	pr_info("gl_test:   %-28s = 0x%08x  want 0x%08x  %s  [check %d]\n",
		what, got, want, ok ? "ok" : "MISMATCH", gl_test_check_no + 1);
	return gl_test_check(ok, got, want);
}

static int case_cubetail(void)
{
	GLuint tex = 0, fbo = 0;
	bool ok = true;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	for (int f = 0; f < 6; f++)
		for (int l = 0; l < 3; l++)
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, l, GL_RGBA, 4 >> l, 4 >> l, 0,
				     GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	if (!check_gl_error("cubetail setup"))
		return -1;
	for (int pass = 0; pass < 2; pass++) {
		for (int i = 0; i < 6; i++) {
			int f = pass == 0 ? i : 5 - i;

			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
					       GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, tex, 2);
			if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
				pr_err("gl_test: cubetail: face %d level 2 incomplete\n", f);
				return -1;
			}
			glViewport(0, 0, 1, 1);
			/* colours exact in 8 bits: k / 255 */
			glClearColor((float)(pass ? 200 : 16 + 32 * f) / 255.0f,
				     (float)(pass ? 16 + 32 * f : 64) / 255.0f,
				     (float)(pass ? 96 : 240 - 32 * f) / 255.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glFinish();
		}
		for (int f = 0; f < 6; f++) {
			uint32_t got = 0;
			uint32_t want = pass ? rgba_u32(200, (uint8_t)(16 + 32 * f), 96, 255)
					     : rgba_u32((uint8_t)(16 + 32 * f), 64, (uint8_t)(240 - 32 * f), 255);
			char what[GL_TEST_LABEL];

			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
					       GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, tex, 2);
			glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &got);
			snprintf(what, sizeof(what), "%s: face %d 1x1", pass ? "order 5..0" : "order 0..5", f);
			ok &= expect_u32(what, got, want);
		}
	}
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glViewport(0, 0, TW, TH);
	return ok && check_gl_error("cubetail teardown") ? 0 : -1;
}

/* ---- case 87: a loop whose break point differs from pixel to pixel --------
 *
 *     for (int i = 0; i < 8; i++) {
 *         if (float(i) >= s * 8.0) break;
 *         if (top > 0.5 && i == 2) continue;
 *         count += 1.0;
 *     }
 *     gl_FragColor = vec4(count / 8.0, 0.0, top > 0.5 ? 1.0 : 0.0, 1.0);
 *
 * Case 81's quad, v_color = (1 - t, t, 1 - s, s). The online PowerVR path
 * keeps this loop as a loop: one body, every exit a flag, and `br if(p0)`
 * on a counter that is the same in every instance, so all instances take the
 * back edge together while each stops counting where its own s says. Band b
 * of eight (left to right) counts b + 1; the top half skips iteration 2, so
 * from band 2 up it counts one fewer. A flag that leaked between instances,
 * a count carried in the wrong register or a back edge taken too often or too
 * rarely each changes a band. Every band's centre is read in both halves.
 *
 * Online-path only, like 68.
 * ------------------------------------------------------------------------- */
static int case_databreak(void)
{
	static const float pos[6][2] = {
		{ -1.0f, -1.0f }, { 1.0f, -1.0f }, { -1.0f, 1.0f },
		{  1.0f, -1.0f }, { 1.0f,  1.0f }, { -1.0f, 1.0f },
	};
	struct pc_vertex v[6];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_passthrough_vs_src, gl_test_databreak_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++) {
		float s = (pos[i][0] + 1.0f) * 0.5f, t = (pos[i][1] + 1.0f) * 0.5f;

		v[i].pos[0] = pos[i][0]; v[i].pos[1] = pos[i][1];
		v[i].pos[2] = 0.0f; v[i].pos[3] = 1.0f;
		v[i].color[0] = 1.0f - t; v[i].color[1] = t;
		v[i].color[2] = 1.0f - s; v[i].color[3] = s;
	}
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("databreak"))
		return -1;
	read_topdown();
	for (int half = 0; half < 2; half++) {
		/* read top-down: half 0 is the top of the quad */
		int top = half == 0;
		int y = top ? TH / 4 : 3 * TH / 4;

		for (int b = 0; b < 8; b++) {
			char what[64];
			int count = b + 1 - (top && b >= 2);
			uint32_t r = (uint32_t)((count * 255 + 4) / 8);
			uint32_t want = 0xff000000u | (top ? 0x00ff0000u : 0) | r;

			snprintf(what, sizeof(what), "databreak: %s band %d counts %d",
				 top ? "top" : "bottom", b, count);
			ok &= expect_px(what, b * TW / 8 + TW / 16, y, want, 2);
		}
	}
	return ok ? 0 : -1;
}

/* ---- case 88: the flip study's resolve-engine arm (GC7000) -----------------
 *
 * Would flipping the default framebuffer at present cost much? The frame
 * clears a 1280x800 FBO green, then its top half (GL y >= 400) red, and is
 * timed to completion twice: with the flip study off, and with
 * gc_debug_flip_experiment = 2, which ends the frame with 100 resolve-engine
 * copies of the target with RS_CONFIG_FLIP set and one plain copy back. The
 * difference over 100 is one flipped copy's cost; the read-back says whether
 * the bit reversed the rows (top green, bottom red) or was ignored (top red).
 * The case passes when the rows are reversed, and prints the cost either way.
 * ------------------------------------------------------------------------- */
#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
extern void gc_debug_set_flip_experiment(int mode);
#endif
static int case_rsflip(void)
{
#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS)
	enum { W = 1280, H = 800 };
	GLuint tex = 0, fbo = 0;
	uint32_t t_off = 0, t_on = 0, px_top = 0, px_bottom = 0;
	bool ok = true;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glViewport(0, 0, W, H);
	for (int arm = 0; arm < 2; arm++) {
		uint32_t t0;

		gc_debug_set_flip_experiment(arm ? 2 : 0);
		t0 = now_ms();
		glDisable(GL_SCISSOR_TEST);
		glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glEnable(GL_SCISSOR_TEST);
		glScissor(0, H / 2, W, H / 2);
		glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDisable(GL_SCISSOR_TEST);
		glFinish();
		if (arm)
			t_on = now_ms() - t0;
		else
			t_off = now_ms() - t0;
	}
	gc_debug_set_flip_experiment(0);
	if (!check_gl_error("rsflip"))
		ok = false;
	/* GL rows: 0 is the bottom */
	glReadPixels(W / 2, H - 8, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &px_top);
	glReadPixels(W / 2, 8, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &px_bottom);
	pr_info("gl_test: rsflip: %ux%u frame %u ms without, %u ms with 100 flipped copies: %u us per copy\n",
		(unsigned)W, (unsigned)H, (unsigned)t_off, (unsigned)t_on,
		t_on > t_off ? (unsigned)((t_on - t_off) * 1000u / 100u) : 0u);
	pr_info("gl_test: rsflip: after the copies top 0x%08x bottom 0x%08x (%s)\n", (unsigned)px_top,
		(unsigned)px_bottom,
		px_top == 0xff00ff00u && px_bottom == 0xff0000ffu ? "rows reversed: RS_CONFIG_FLIP flips" :
		px_top == 0xff0000ffu && px_bottom == 0xff00ff00u ? "rows as drawn: RS_CONFIG_FLIP ignored" :
		"neither: see the pixels");
	ok &= px_top == 0xff00ff00u && px_bottom == 0xff0000ffu;
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glViewport(0, 0, TW, TH);
	return ok ? 0 : -1;
#elif defined(GL_TEST_VIVANTE)
	pr_info("gl_test: rsflip: the flip study is a test knob (CONFIG_YAKOGL_TEST_KNOBS)\n");
	return -2;
#else
	pr_info("gl_test: rsflip: the resolve engine is the GC7000's\n");
	return -2;
#endif
}

/* ---- case 89: flat varyings take the last vertex's value -------------------
 *
 * One triangle covering the target, its vertices red / green / blue with
 * alpha 0.25 / 0.5 / 0.75. The vertex shader passes the colour as `flat out
 * vec4` and uint(alpha * 4 + 0.5) = 1 / 2 / 3 as `flat out uint`; the fragment
 * shader writes (colour.rgb, uint / 4). GL's provoking vertex is the LAST (ES
 * 3.0 2.18), so every pixel is blue with alpha 0.75: 0xbfff0000. The FIRST
 * vertex's convention gives red, alpha 0.25 (0x400000ff); a varying that is
 * interpolated after all gives a colour that changes across the triangle.
 * Blending is off, so alpha is stored as written.
 * ------------------------------------------------------------------------- */
static int case_flatlast(void)
{
	static const float v[3][8] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f,   1.0f, 0.0f, 0.0f, 0.25f },
		{  3.0f, -1.0f, 0.0f, 1.0f,   0.0f, 1.0f, 0.0f, 0.50f },
		{ -1.0f,  3.0f, 0.0f, 1.0f,   0.0f, 0.0f, 1.0f, 0.75f },
	};
	static const struct { int x, y; } at[3] = { { TW / 2, TH / 2 }, { 4, 4 }, { TW - 5, TH - 5 } };
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_es300_flatlast_vs_src, gl_test_es300_flatlast_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 8 * sizeof(float), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 8 * sizeof(float), (const void *)(4 * sizeof(float)));
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glDisable(GL_BLEND);
	glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	if (!check_gl_error("flatlast"))
		return -1;
	read_topdown();
	for (int i = 0; i < 3; i++)
		ok &= expect_px("flatlast: the last vertex's flat colour and uint", at[i].x, at[i].y, 0xbfff0000, 2);
	return ok ? 0 : -1;
}

/* ---- case 91: blending into an RGBA16F target, above 1.0 ------------------
 *
 * An 8x8 RGBA16F texture, cleared to (2.0, 0.5, 0.25, 1.0), takes one colour
 * u = (0.5, 1.5, 0.0, 0.5) twice in one frame, a half each (scissored):
 *
 *   left   ONE, ONE_MINUS_SRC_ALPHA        (Quick 3D's SourceOver, what its
 *          layers with post-processing blend into RGBA16F with)
 *          u + dst * 0.5            = (1.5, 1.75, 0.125, 1.0)
 *   right  SRC_ALPHA, ONE_MINUS_SRC_ALPHA
 *          u * 0.5 + dst * 0.5      = (1.25, 1.0, 0.125, 0.75)
 *
 * Every value is a half exactly. The texture is then drawn over the window
 * at a quarter scale (es100/hdrscale.frag), so the halves read (96, 112, 8,
 * 64) and (80, 64, 8, 48). The PowerVR has no fixed-function blend: the
 * fragment shader reads the pixel and writes the result, and for RGBA16F the
 * pixel is two dwords of halves. What each failure reads as:
 *   - the source or the pixel clamped to 1 (the RGBA8 variant's fsat, or a
 *     UNORM read): left green 1.25 -> 80, not 112;
 *   - no blend at all (the replace variant): left (32, 96, 0, 32);
 *   - the second dword (blue, alpha) not read or not written: blue 16 or 0.
 * Host prediction: tests/usc_limits.rs
 * gl_test_case_91_blends_into_rgba16f_on_the_host (the byte model, the same
 * variant the GL core asks for, the same bytes).
 *
 * Online only: the offline table's RGBA16F blend rows are Quick
 * 3D's shaders, not this one.
 * ------------------------------------------------------------------------- */
static int case_hdrblend(void)
{
	GLuint cprog, sprog, tex = 0, fbo = 0;
	bool ok = true;

	free_all_objects();
	cprog = link_program(gl_test_hdrquad_vs_src, gl_test_hdrcolor_fs_src);
	sprog = link_program(gl_test_hdrquad_vs_src, gl_test_hdrscale_fs_src);
	if (!cprog || !sprog)
		return -1;

	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F, 8, 8, 0, GL_RGBA, GL_HALF_FLOAT, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		pr_err("gl_test: hdrblend: RGBA16F FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, 8, 8);
	glClearColor(2.0f, 0.5f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glUseProgram(cprog);
	glUniform4f(glGetUniformLocation(cprog, "u_color"), 0.5f, 1.5f, 0.0f, 0.5f);
	puv_quad_setup();
	glEnable(GL_BLEND);
	glEnable(GL_SCISSOR_TEST);
	glScissor(0, 0, 4, 8);
	glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glScissor(4, 0, 4, 8);
	glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisable(GL_SCISSOR_TEST);
	glDisable(GL_BLEND);
	glFinish();
	if (!check_gl_error("hdrblend: blended draws"))
		return -1;

	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	glUseProgram(sprog);
	glUniform1i(glGetUniformLocation(sprog, "hdr"), 0);
	puv_quad_setup();
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("hdrblend: read-back draw"))
		return -1;
	read_topdown();
	for (uint32_t y = TH / 4; y < TH; y += TH / 2) {
		ok &= expect_px("hdrblend: left, one/one_minus_src_alpha x0.25", TW / 4, y,
				rgba_u32(96, 112, 8, 64), 2);
		ok &= expect_px("hdrblend: right, src_alpha/one_minus_src_alpha x0.25", 3 * TW / 4, y,
				rgba_u32(80, 64, 8, 48), 2);
	}

	/* Which half failed, when checks 1-4 do (the i.MX8MP read 0x00000000 at
	 * check 1, alpha 0 too: the scale draw sampled zero everywhere). Added
	 * after them so they keep their numbers:
	 *   5-6: the RGBA16F target read back directly as floats -- did the
	 *        clear and the blends reach its memory? Each value is reported
	 *        as v * 64 in a byte, so the wanted colours are checks 1-4's.
	 *   7:   a CPU-uploaded RGBA16F texel (0.5, 0.25, 1.0, 1.0), drawn at a
	 *        quarter scale -- does sampling RGBA16F work at all? */
	{
		static GLfloat px[8 * 8 * 4];
		uint32_t want[2] = { rgba_u32(96, 112, 8, 64), rgba_u32(80, 64, 8, 48) };
		static const uint16_t texel[4] = { 0x3800, 0x3400, 0x3c00, 0x3c00 };
		GLuint up = 0;

		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glReadPixels(0, 0, 8, 8, GL_RGBA, GL_FLOAT, px);
		for (int k = 0; k < 2; k++) {
			const GLfloat *v = &px[(4 * 8 + (k ? 6 : 1)) * 4];
			uint32_t got = rgba_u32((uint8_t)(v[0] * 64.0f + 0.5f), (uint8_t)(v[1] * 64.0f + 0.5f),
						(uint8_t)(v[2] * 64.0f + 0.5f), (uint8_t)(v[3] * 64.0f + 0.5f));
			char what[GL_TEST_LABEL];

			snprintf(what, sizeof(what), "hdrblend: RGBA16F memory, %s, x64", k ? "right" : "left");
			pr_info("gl_test:   %s floats (%g, %g, %g, %g)\n", what, (double)v[0], (double)v[1],
				(double)v[2], (double)v[3]);
			ok &= expect_rgb_tol(what, got, want[k], 2);
		}
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		if (!check_gl_error("hdrblend: float read-back"))
			return -1;

		glGenTextures(1, &up);
		glBindTexture(GL_TEXTURE_2D, up);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F, 1, 1, 0, GL_RGBA, GL_HALF_FLOAT, texel);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glViewport(0, 0, TW, TH);
		glUseProgram(sprog);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("hdrblend: uploaded texel draw"))
			return -1;
		read_topdown();
		/* (0.5, 0.25, 1.0, 1.0) x 0.25 */
		ok &= expect_px("hdrblend: an uploaded RGBA16F texel x0.25", TW / 2, TH / 2, rgba_u32(32, 16, 64, 64), 2);
		glDeleteTextures(1, &up);
	}

	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("hdrblend teardown") ? 0 : -1;
}

/* ---- case 97: fifteen vec4 varyings, every io row the GC7000 reports -------
 *
 * GL_MAX_VARYING_VECTORS is 15 on the GC7000 since 76f3c5d (it was 8, and a
 * ninth vec4 varying panicked in the draw until 6719382). The vertex shader
 * writes fifteen distinct constants, four different lanes each; the fragment
 * shader checks every one and writes green (0xff00ff00) when all arrive, red
 * with blue = (index of the first wrong one + 1) / 16 otherwise, so a
 * failure names its varying. Constants, so interpolation cannot move them:
 * only a lost, swapped or misprogrammed row can. Skipped by name on a GPU
 * that reports fewer than fifteen (the PowerVR's 8 until usc-backend's
 * measurement raises it).
 * ------------------------------------------------------------------------- */
static int case_varyings15(void)
{
	static const float pos[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const struct { int x, y; } at[4] = { { 4, 4 }, { TW - 5, 4 }, { TW / 2, TH / 2 }, { 4, TH - 5 } };
	GLint maxv = 0;
	GLuint prog;
	bool ok = true;

	glGetIntegerv(GL_MAX_VARYING_VECTORS, &maxv);
	if (maxv < 15) {
		pr_info("gl_test: case 97 varyings15: SKIP (GL_MAX_VARYING_VECTORS %d < 15)\n", (int)maxv);
		return -3;
	}
	prog = link_program(gl_test_varyings15_vs_src, gl_test_varyings15_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	upload_vbo(pos, sizeof(pos));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("varyings15"))
		return -1;
	read_topdown();
	for (int i = 0; i < 4; i++) {
		uint32_t px = s_px[at[i].y * TW + at[i].x];

		if ((px & 0xff) == 0xff && (px & 0xff00) == 0)
			pr_info("gl_test:   varyings15: (%d, %d) = 0x%08x, the first wrong varying is v%u\n", at[i].x,
				at[i].y, px, ((((px >> 16) & 0xff) * 16 + 127) / 255) - 1);
		ok &= expect_px("varyings15: all fifteen arrive", at[i].x, at[i].y, 0xff00ff00, 2);
	}
	return ok ? 0 : -1;
}

/* ---- case 101: a cube map with a bias (texldb) and at explicit levels ------
 *
 * texldb has no instance in the reference table, so the GC7000's biased cube
 * look-up (f3eeea0) is checked here rather than left to dEQP. A 64x64 cube
 * map, seven levels, every face of level l one colour: red, green, blue,
 * yellow, magenta, cyan, white for 0..6. NEAREST_MIPMAP_NEAREST, so a level
 * is either hit or visibly missed.
 *
 *   bias: the fragment shader samples the +X face with face coordinates that
 *         span the face across the 64-pixel target -- one texel per pixel at
 *         level 0, an implicit level of detail of exactly 0 -- plus the bias:
 *         0, +1, +2 must read levels 0, 1, 2 (red, green, blue). Integer
 *         levels, far from the rounding point between two.
 *   lod:  the vertex shader samples +X at the explicit levels 0, 1, 2, 3
 *         (textureCubeLod, texldl) and passes the texel on: red, green,
 *         blue, yellow.
 * Each reading is printed with the level its colour names, so a miss says
 * which level was hit instead.
 * ------------------------------------------------------------------------- */
static int cube_level_of(uint32_t px)
{
	static const uint32_t c[7] = { 0xff0000ff, 0xff00ff00, 0xffff0000, 0xff00ffff,
				       0xffff00ff, 0xffffff00, 0xffffffff };

	for (int l = 0; l < 7; l++)
		if (px == c[l])
			return l;
	return -1;
}

static int case_cubebias(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const uint8_t rgb[7][3] = { { 255, 0, 0 }, { 0, 255, 0 }, { 0, 0, 255 }, { 255, 255, 0 },
					   { 255, 0, 255 }, { 0, 255, 255 }, { 255, 255, 255 } };
	static uint32_t level[64 * 64];
	GLuint tex = 0, prog;
	bool ok = true;

	glGenTextures(1, &tex);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	for (int l = 0; l < 7; l++) {
		int n = 64 >> l;

		for (int i = 0; i < n * n; i++)
			level[i] = rgba_u32(rgb[l][0], rgb[l][1], rgb[l][2], 255);
		for (int f = 0; f < 6; f++)
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, l, GL_RGBA, n, n, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, level);
	}
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("cubebias setup"))
		return -1;
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);

	prog = link_program(gl_test_cubebias_vs_src, gl_test_cubebias_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_s"), 0);
	for (int b = 0; b <= 2; b++) {
		uint32_t px;
		char what[GL_TEST_LABEL];

		glUniform1f(glGetUniformLocation(prog, "u_bias"), (float)b);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		px = s_px[(TH / 2) * TW + TW / 2];
		pr_info("gl_test:   cubebias: bias %d reads level %d (0x%08x)\n", b, cube_level_of(px), px);
		snprintf(what, sizeof(what), "cubebias: bias %d -> level %d", b, b);
		ok &= expect_px(what, TW / 2, TH / 2, rgba_u32(rgb[b][0], rgb[b][1], rgb[b][2], 255), 0);
	}

	prog = link_program(gl_test_cubelod_vs_src, gl_test_cubelod_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "u_s"), 0);
	for (int l = 0; l <= 3; l++) {
		uint32_t px;
		char what[GL_TEST_LABEL];

		glUniform1f(glGetUniformLocation(prog, "u_lod"), (float)l);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		px = s_px[(TH / 2) * TW + TW / 2];
		pr_info("gl_test:   cubebias: textureCubeLod %d reads level %d (0x%08x)\n", l, cube_level_of(px), px);
		snprintf(what, sizeof(what), "cubebias: lod %d -> level %d", l, l);
		ok &= expect_px(what, TW / 2, TH / 2, rgba_u32(rgb[l][0], rgb[l][1], rgb[l][2], 255), 0);
	}
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("cubebias") ? 0 : -1;
}

/* ---- case 114: a cube sampled at a high explicit level, whole face ---------
 *
 * The emulated cube keeps its six faces stacked in one 2D image, and the
 * look-up keeps uv half a texel inside the face. That half texel was taken
 * at LEVEL 0 (glsl2usc's pvr_cubeUV, and the online Sel::cube_uv): at a
 * level where a face is a texel or two tall it clamps nothing, and a
 * bilinear filter blends in the rows of the face stacked next to it -- dEQP
 * texture_functions.vertex.texturecubelod drew a gradient across its
 * high-LOD corner on the AM62P (am62p-board, 30899c1f). Case 101 samples
 * only the centre of +X and cannot see it.
 *
 * A 16x16 cube, five levels, a colour per face AND per level (red 50*face,
 * green 50*level, blue 200), LINEAR within a level. textureLod at levels 3
 * (a 2x2 face), 4 (1x1) and 6 (past the chain: level 4), in directions
 * across the whole +X face, once per vertex and once per pixel: every pixel
 * must read the level's flat +X colour; -X, stacked below it, is red 50.
 *
 * Judged per filter: NEAREST always (it never reaches another face, on
 * either cube). LINEAR only in an OpenGL ES 2.0 context, whose cube maps
 * are not seamless (the PowerVR sets SAMPLER_WORD1.cemedge_dontfilter in a
 * report-es2 build). ES 3.0 requires seamless filtering: at a 2x2 or 1x1
 * face a LINEAR footprint legitimately takes in the neighbouring faces, and
 * with the native cube (the default since step C, 15550645) that is what
 * the AM62P's ES3 pairs drew (red 0x6c-0x72 from the other faces), which
 * the case once judged as a leak. LINEAR is still drawn and printed there.
 * ES 3.00 in both stages (textureLod); in the table as well as online.
 * ------------------------------------------------------------------------- */
static int case_cubelod(void)
{
	static const float quad[6][4] = {
		{ -1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f, -1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
		{  1.0f, -1.0f, 0.0f, 1.0f }, { 1.0f,  1.0f, 0.0f, 1.0f }, { -1.0f, 1.0f, 0.0f, 1.0f },
	};
	static const int lods[3] = { 3, 4, 6 };
	static uint32_t level[16 * 16];
	const char *stage[2] = { "vertex", "fragment" };
	GLuint tex = 0, prog[2];
	bool ok = true;

	free_all_objects();
	glGenTextures(1, &tex);
	glActiveTexture(GL_TEXTURE0);
	glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
	for (int l = 0; l < 5; l++) {
		int n = 16 >> l;

		for (int f = 0; f < 6; f++) {
			for (int i = 0; i < n * n; i++)
				level[i] = rgba_u32(50 * f, 50 * l, 200, 255);
			glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, l, GL_RGBA, n, n, 0, GL_RGBA,
				     GL_UNSIGNED_BYTE, level);
		}
	}
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!check_gl_error("cubelod setup"))
		return -1;
	upload_vbo(quad, sizeof(quad));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (const void *)0);
	glEnableVertexAttribArray(0);
	glDisable(GL_BLEND);
	glViewport(0, 0, TW, TH);

	prog[0] = link_program(gl_test_es300_cubelodv_vs_src, gl_test_es300_cubelodv_fs_src);
	prog[1] = link_program(gl_test_es300_cubelodf_vs_src, gl_test_es300_cubelodf_fs_src);
	if (!prog[0] || !prog[1])
		return -1;
	const char *ver = (const char *)glGetString(GL_VERSION);
	bool seamful = ver && strstr(ver, "OpenGL ES 2") != NULL;
	for (int fl = 0; fl < 2; fl++) {
	bool judge = fl == 1 || seamful;
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, fl ? GL_NEAREST_MIPMAP_NEAREST : GL_LINEAR_MIPMAP_NEAREST);
	glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, fl ? GL_NEAREST : GL_LINEAR);
	for (int st = 0; st < 2; st++) {
		glUseProgram(prog[st]);
		glUniform1i(glGetUniformLocation(prog[st], "u_s"), 0);
		for (int k = 0; k < 3; k++) {
			int lv = lods[k] < 4 ? lods[k] : 4;
			uint32_t want = rgba_u32(0, 50 * lv, 200, 255);
			uint32_t x = TW / 2, y = TH / 2, bad = 0;
			char what[GL_TEST_LABEL];

			glUniform1f(glGetUniformLocation(prog[st], "u_lod"), (float)lods[k]);
			glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
			read_topdown();
			/* every pixel: the first that is not the flat colour is the
			 * one reported, and how many there are */
			for (uint32_t i = 0; i < TW * TH; i++) {
				if (chan_diff(s_px[i], want) > 1) {
					if (bad++ == 0) {
						x = i % TW;
						y = i / TW;
					}
				}
			}
			pr_info("gl_test:   cubelod: %s %s lod %d: %u of %u pixels off level %d's +X colour%s\n",
				fl ? "NEAREST" : "LINEAR", stage[st], lods[k], bad, TW * TH, lv,
				judge ? "" : " (seamless ES 3.0: not judged)");
			snprintf(what, sizeof(what), "cubelod: %s %s lod %d", fl ? "nearest" : "linear", stage[st], lods[k]);
			if (judge)
				ok &= expect_px(what, x, y, want, 1);
		}
	}
	}
	glDeleteTextures(1, &tex);
	return ok && check_gl_error("cubelod") ? 0 : -1;
}

/* ---- case 98: a loop bounded by a uniform, counted at run time -------------
 *
 *     for (int i = 0; float(i) < count; i++) { n += 1.0; last = i; }
 *
 * es300/uniloop.frag, Qt Quick 3D's light-loop shape (an ES 3.00 loop whose
 * bound is a uniform). The online PowerVR path keeps it as one loop whose
 * back edge is taken while the counter is below UNIFORM_LOOP_CAP (4096) and
 * the uniform exit flag is clear, so every instance leaves together when the
 * bound says. Four draws, one per quadrant, with count 0, 1, 7 and 4097; the
 * colour writes n in steps of 16 (red the ones, green the sixteens, blue the
 * 256s) and is white if the counter carried out of the loop disagrees with n.
 *
 * 4097 is past the cap: the loop stops after 4096, a DELIBERATE deviation
 * (unroll.rs UNIFORM_LOOP_CAP, docs/online-shaderc/usc-seam.md), so the
 * quadrant reads blue 240 and red 0 where GL would give red 16. That it
 * finishes at all is the other half: a back edge that ignored its cap or its
 * flag hangs or runs long, and a flag that ignored the bound reads 4096 in
 * every quadrant. Colours host-checked by usc_limits
 * gl_test_case_98_counts_what_the_board_case_expects.
 *
 * Online-path only, like 68 (the offline table has no blob for it).
 * ------------------------------------------------------------------------- */
static int case_uniloop(void)
{
	static const float counts[4] = { 0.0f, 1.0f, 7.0f, 4097.0f };
	/* AABBGGRR, as the host test pins them */
	static const uint32_t want[4] = { 0xff000000u, 0xff000010u, 0xff000070u, 0xfff00000u };
	GLuint prog;
	GLint loc;
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_vid_vs_src, gl_test_es300_uniloop_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "count");
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	/* green: no expected colour has any, so an undrawn quadrant cannot pass
	 * for count 0 */
	glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	for (int q = 0; q < 4; q++) {
		glViewport((q & 1) * (TW / 2), (q >> 1) * (TH / 2), TW / 2, TH / 2);
		glUniform1f(loc, counts[q]);
		glDrawArrays(GL_TRIANGLES, 0, 6);
	}
	glFinish();
	glViewport(0, 0, TW, TH);
	if (!check_gl_error("uniloop"))
		return -1;
	read_topdown();
	for (int q = 0; q < 4; q++) {
		char what[64];
		/* GL's quadrant q starts at y = (q >> 1) * TH / 2 from the bottom */
		uint32_t x = (q & 1) * (TW / 2) + TW / 4;
		uint32_t y = (q >> 1) ? TH / 4 : 3 * TH / 4;

		snprintf(what, sizeof(what), "uniloop: count %g", (double)counts[q]);
		ok &= expect_px(what, x, y, want[q], 3);
	}
	return ok && check_gl_error("uniloop teardown") ? 0 : -1;
}

/* ---- case 102: a colour clear under a partial glColorMask -------------------
 *
 * GL ES 2.0 4.2.3: glClear writes the buffers "enabled for writing", and
 * glColorMask applies to it channel by channel. dEQP-GLES2.functional.
 * color_clear's masked_* / masked_scissored_* / complex_* / long_masked_*
 * cases clear with random partial masks. With the switch off a partially
 * masked clear writes all four channels here; this case measures what that
 * costs, directly, where dEQP could not (its masked cases passed in the
 * group run on the i.MX8MP, dafa9962, and masked_rgb alone failed).
 *
 * Three shapes, each checked over the WHOLE image, not sampled:
 *   1. full screen, R and A on: (0,0,0,1) under (1,1,1,0.5) -> (1,0,0,0.5)
 *   2. scissored to the bottom-left 32x32, G only: black under (0,1,0,0)
 *      -> green inside, black outside
 *   3. fused into a draw: B only, the mask set back to all before a draw
 *      covering the bottom-left quadrant yellow; the clear keeps its own
 *      mask -> blue elsewhere, yellow in the quadrant
 * GC7000: passes (the clear quad programs the mask; with every channel
 * written, the switch the clean-up removed, every shape failed). PowerVR:
 * FAILS until its clear shader has a masked variant (usc-backend) -- a known
 * defect shows as a failure, with the reason on the line after it.
 * ------------------------------------------------------------------------- */

/* Every pixel of the last read_topdown(): `in` inside the top-down box
 * [x0,x1) x [y0,y1), `out` elsewhere. Reports the count and the first bad
 * pixel, so a partial write reads as a number, not a sample. */
static bool expect_box(const char *what, uint32_t x0, uint32_t y0, uint32_t x1, uint32_t y1,
		       uint32_t in, uint32_t out, int tol)
{
	uint32_t bad = 0, first = 0, first_want = 0;

	for (uint32_t y = 0; y < TH; y++)
		for (uint32_t x = 0; x < TW; x++) {
			bool inside = x >= x0 && x < x1 && y >= y0 && y < y1;

			if (chan_diff(s_px[y * TW + x], inside ? in : out) > tol) {
				if (!bad) {
					first = y * TW + x;
					first_want = inside ? in : out;
				}
				bad++;
			}
		}
	pr_info("gl_test:   %-28s %u of %u pixels wrong%s\n", what, bad, TW * TH, bad ? "  MISMATCH" : "  ok");
	if (bad)
		pr_info("gl_test:   first wrong (%u,%u) = 0x%08x\n", first % TW, first / TW, s_px[first]);
	return gl_test_check(bad == 0, s_px[first], first_want);
}

static int case_maskclear(void)
{
	struct cuv_vertex q[6];
	GLuint prog;
	bool ok = true;
	static const float c[6][2] = {
		{ -1, -1 }, { 0, -1 }, { 0, 0 }, { -1, -1 }, { 0, 0 }, { -1, 0 },
	};

	glDisable(GL_SCISSOR_TEST);
	glDisable(GL_BLEND);

	/* 1. full screen, R and A */
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glColorMask(GL_TRUE, GL_FALSE, GL_FALSE, GL_TRUE);
	glClearColor(1.0f, 1.0f, 1.0f, 0.5f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	if (!check_gl_error("clear 1"))
		return -1;
	read_topdown();
	ok &= expect_box("R+A: (1,0,0,0.5)", 0, 0, TW, TH, 0x800000ff, 0x800000ff, 1);

	/* 2. scissored, G only */
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glEnable(GL_SCISSOR_TEST);
	glScissor(0, 0, 32, 32);
	glColorMask(GL_FALSE, GL_TRUE, GL_FALSE, GL_FALSE);
	glClearColor(0.0f, 1.0f, 0.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	if (!check_gl_error("clear 2"))
		return -1;
	read_topdown();
	/* GL's bottom-left 32x32 is the top-down image's rows 32..63 */
	ok &= expect_box("G, scissored", 0, TH / 2, TW / 2, TH, 0xff00ff00, 0xff000000, 0);

	/* 3. fused into a draw, B only */
	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	for (int i = 0; i < 6; i++)
		cuv_set(&q[i], c[i][0], c[i][1], 0.0f, 1, 1, 0);
	upload_vbo(q, sizeof(q));
	cuv_attribs();
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glColorMask(GL_FALSE, GL_FALSE, GL_TRUE, GL_FALSE);
	glClearColor(0.0f, 0.0f, 1.0f, 0.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	if (!check_gl_error("clear 3"))
		return -1;
	read_topdown();
	ok &= expect_box("B, fused; quad yellow", 0, TH / 2, TW / 2, TH, 0xff00ffff, 0xffff0000, 0);
	if (!ok) {
#if defined(GL_TEST_PVR)
		pr_info("gl_test:   maskclear: known defect: the PowerVR clear shader has no masked variant, so every clear writes all four channels (usc-backend)\n");
#endif
	}
	return ok ? 0 : -1;
}

/* ---- case 100: a texture and a dynamically indexed uniform array, both stages
 *
 * es300/idxtex.vert and idxtex.frag: each stage samples its own 2x2 texture
 * (the vertex stage with textureLod) and reads a uniform array by a uniform
 * int. A stage's sampler descriptors come before its uniform block, so the
 * indexed read must be addressed past them; the online PowerVR path read it
 * 20 dwords too low until 18ae1599 (Qt's morph targets). Red and green are
 * the vertex stage's 0.5 * (vcols[vidx] + texel), blue and alpha the
 * fragment stage's 0.5 * (fcols[fidx] + texel), each read back for two index
 * pairs. Host-checked by usc_limits gl_test_case_100_reads_past_the_
 * descriptors_in_both_stages (both stages against the IR; the vertex stage
 * fails it without the fix).
 *
 * Online-path only, like 68 (the offline table has no blob for it).
 * ------------------------------------------------------------------------- */
static int case_idxtex(void)
{
	static const uint8_t vtexels[16] = {
		204, 51, 0, 255,   0, 0, 0, 255,
		0, 0, 0, 255,      0, 0, 0, 255,
	};
	static const uint8_t ftexels[16] = {
		0, 0, 0, 0,        0, 0, 0, 0,
		0, 0, 0, 0,        0, 0, 102, 153,
	};
	static const float vcols[4][4] = {
		{ 0.2f, 0.6f, 0.0f, 0.0f }, { 0.8f, 0.2f, 0.0f, 0.0f },
		{ 0.4f, 0.4f, 0.0f, 0.0f }, { 0.0f, 1.0f, 0.0f, 0.0f },
	};
	static const float fcols[4][4] = {
		{ 0.0f, 0.0f, 0.2f, 0.8f }, { 0.0f, 0.0f, 0.6f, 0.2f },
		{ 0.0f, 0.0f, 1.0f, 0.0f }, { 0.0f, 0.0f, 0.4f, 0.6f },
	};
	static const int pairs[2][2] = { { 1, 2 }, { 3, 0 } };
	GLuint prog, tex[2];
	bool ok = true;

	free_all_objects();
	prog = link_program(gl_test_es300_idxtex_vs_src, gl_test_es300_idxtex_fs_src);
	if (!prog)
		return -1;
	glGenTextures(2, tex);
	for (int t = 0; t < 2; t++) {
		glActiveTexture(GL_TEXTURE0 + t);
		glBindTexture(GL_TEXTURE_2D, tex[t]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE,
			     t == 0 ? vtexels : ftexels);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	}
	glActiveTexture(GL_TEXTURE0);
	glUseProgram(prog);
	glUniform1i(glGetUniformLocation(prog, "vtex"), 0);
	glUniform1i(glGetUniformLocation(prog, "ftex"), 1);
	glUniform4fv(glGetUniformLocation(prog, "vcols"), 4, &vcols[0][0]);
	glUniform4fv(glGetUniformLocation(prog, "fcols"), 4, &fcols[0][0]);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	for (int k = 0; k < 2; k++) {
		int vi = pairs[k][0], fi = pairs[k][1];
		float c[4] = {
			0.5f * (vcols[vi][0] + vtexels[0] / 255.0f),
			0.5f * (vcols[vi][1] + vtexels[1] / 255.0f),
			0.5f * (fcols[fi][2] + ftexels[14] / 255.0f),
			0.5f * (fcols[fi][3] + ftexels[15] / 255.0f),
		};
		char what[64];

		glUniform1i(glGetUniformLocation(prog, "vidx"), vi);
		glUniform1i(glGetUniformLocation(prog, "fidx"), fi);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!check_gl_error("idxtex draw"))
			return -1;
		read_topdown();
		snprintf(what, sizeof(what), "idxtex: vidx %d fidx %d", vi, fi);
		ok &= expect_px(what, TW / 2, TH / 2,
				rgba_u32((uint8_t)(c[0] * 255.0f + 0.5f), (uint8_t)(c[1] * 255.0f + 0.5f),
					 (uint8_t)(c[2] * 255.0f + 0.5f), (uint8_t)(c[3] * 255.0f + 0.5f)), 3);
	}
	glDeleteTextures(2, tex);
	return ok && check_gl_error("idxtex teardown") ? 0 : -1;
}

/* ---- case 104: loops whose trip counts differ from pixel to pixel ------------
 *
 * Two halves of one target, each band b of eight (left to right) looping
 * per pixel, so instances of one USC task leave at different passes -- run
 * under pco's execution mask (cndst/cndsm/cndlt and a back edge taken while
 * any instance is still in), which the host models for one instance only.
 *
 * Top half, es300/divloop.frag: band b loops b times -- band 0 never enters,
 * band 7's bound (1e9) stops at the 4096 cap (a DELIBERATE deviation; GL
 * would never end). Colour: the count in steps of 16 per channel (red the
 * ones, green the sixteens, blue the 256s), white if the loop counter
 * disagrees with it.
 *
 * Bottom half, es300/divnest.frag: a loop inside a loop, both per pixel,
 * with a continue and a break at each level (the outer skips i == 2 and
 * leaves once n >= 6, the inner skips j == 1 and leaves at j >= b - 2).
 * Colour: n as above; alpha the outer loop's passes in steps of 32. The
 * expected values are recomputed here by running the same loops in C.
 *
 * Cleared green, which no expected colour has. Predicted on the host by
 * usc_limits gl_test_case_104_counts_what_the_board_case_expects: top band b
 * = 0..6 red 16*b, band 7 blue 240; bottom n = 0,0,0,2,3,6,6,8 and outer
 * passes 0,1,2,3,4,4,2,2.
 * A GPU hang is the failure this case can cause: run it alone first, with the
 * recovery count reported.
 *
 * Online-path only, like 68 (the offline table has no blob for it).
 * ------------------------------------------------------------------------- */
static uint32_t divloop_colour(uint32_t n, uint32_t a)
{
	return rgba_u32((uint8_t)((n % 16) * 16), (uint8_t)(((n / 16) % 16) * 16),
			(uint8_t)((n / 256 > 15 ? 15 : n / 256) * 16), (uint8_t)a);
}

/* divnest.frag's two loops, for band b: n and the outer loop's passes */
static void divnest_expect(int b, uint32_t *n_out, uint32_t *outer_out)
{
	uint32_t n = 0, outer = 0;

	for (int i = 0; i < b; i++) {
		outer = (uint32_t)i + 1;
		if (i == 2)
			continue;
		for (int j = 0; j < b; j++) {
			if (j == 1)
				continue;
			if (j >= b - 2)
				break;
			n++;
		}
		if (n >= 6)
			break;
	}
	*n_out = n;
	*outer_out = outer;
}

static int case_divloop(void)
{
	GLuint flat, nest;
	bool ok = true;

	free_all_objects();
	flat = link_program(gl_test_es300_bands_vs_src, gl_test_es300_divloop_fs_src);
	nest = link_program(gl_test_es300_bands_vs_src, gl_test_es300_divnest_fs_src);
	if (!flat || !nest)
		return -1;
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	/* GL's top half (y from TH/2) the single loop, the bottom half the nest */
	glUseProgram(flat);
	glViewport(0, TH / 2, TW, TH / 2);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glUseProgram(nest);
	glViewport(0, 0, TW, TH / 2);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish();
	glViewport(0, 0, TW, TH);
	if (!check_gl_error("divloop"))
		return -1;
	read_topdown();
	for (int b = 0; b < 8; b++) {
		char what[64];
		uint32_t x = b * TW / 8 + TW / 16;
		uint32_t n = b == 7 ? 4096u : (uint32_t)b, nn, outer;

		snprintf(what, sizeof(what), "divloop: band %d loops %u", b, n);
		ok &= expect_px(what, x, TH / 4, divloop_colour(n, 255), 3);
		divnest_expect(b, &nn, &outer);
		snprintf(what, sizeof(what), "divloop nested: band %d n %u outer %u", b, nn, outer);
		ok &= expect_px(what, x, 3 * TH / 4, divloop_colour(nn, outer * 32), 3);
	}
	return ok && check_gl_error("divloop teardown") ? 0 : -1;
}

/* ---- case 107: a colour clear after a frame began by loading colour --------
 * A frame that begins without a colour clear first reloads the target's old
 * colour; a colour clear arriving after that, made the frame's background,
 * was painted over by the reload (AM62P 2026-09-24: dEQP color_clear's
 * scissored cases, whose framework clear under a box missing the target
 * opened the frame). Two ways in, each read back over every pixel:
 *   1. a clear whose scissor box misses the target, then a full clear;
 *   2. glColorMask(0,0,0,0) + glClear(COLOR|DEPTH), then a colour clear. */
static int case_clearload(void)
{
	bool ok = true;

	glDisable(GL_BLEND);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);

	/* 1. */
	glDisable(GL_SCISSOR_TEST);
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glEnable(GL_SCISSOR_TEST);
	glScissor(-200, 0, 50, 50);            /* wholly left of the target */
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glScissor(0, 0, TW, TH);
	glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDisable(GL_SCISSOR_TEST);
	if (!check_gl_error("missed box, then full clear"))
		return -1;
	read_topdown();
	ok &= expect_box("missed box, then full clear", 0, 0, TW, TH, 0xff00ff00, 0xff00ff00, 0);

	/* 2. */
	glClearColor(1.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glColorMask(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glClearColor(0.0f, 1.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	if (!check_gl_error("masked clear, then colour clear"))
		return -1;
	read_topdown();
	ok &= expect_box("masked clear, then colour", 0, 0, TW, TH, 0xff00ff00, 0xff00ff00, 0);
	return ok ? 0 : -1;
}

#ifdef GL_TEST_PVR
/* ---- case 174: the lost FWCCB command, reproduced (PowerVR) ----------------
 *
 * The AM62P glmark2 cell's named defect "FWCCB slot read empty (a lost
 * command)": the firmware asks for a freelist grow on the FWCCB, the write
 * offset says the slot is written, and the host reads type 0 there (with
 * CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK: "slot S read empty; still empty after
 * 2000 us"). glmark2 met it at slot 12, read offset 12 / write offset 14,
 * the 13th command of a run (driver/docs/pvr-render-pool.md). The loss is in
 * the queue's traffic, so this case makes traffic that the driver always
 * answers with pages:
 * - frames of L layers into a 1024x1024 target, each frame flushed without
 *   waiting, so two frames' geometry is in flight. A layer is one draw of a
 *   4x4-pixel quad inside each of the 1024 32x32 tiles: it costs the tiler
 *   parameter memory in every tile and the shader almost nothing. (v1 drew
 *   full-screen quads: about 1 ms of GPU time each, so the frames that ask
 *   took seconds.);
 * - L up by an eighth every phase of 16 frames (fwl_next), held once a
 *   phase asked in three quarters of its frames (v6, with
 *   SAFEUI_GPU_PVR_FL_PROBE_SERVE_MAX: three served grows, then a refusal at
 *   nearly every frame, as glmark2's refract; v4 held after the third
 *   served grow and v5 at the first refusal, and in both the following
 *   frames fit and nothing asked again) or once FWL_HOLD_GROWS grows were
 *   served (the list at about 512 + 10 x 256 = 3072 of 4096
 *   pages) or at FWL_MAXL. v2 kept rising and reached the maximum at 97
 *   layers in 3.7 s: every run ended in the at-maximum stop (cause 2),
 *   which is the runaway's signature and must stay that alone;
 * - built with CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND (and no probe), every
 *   grow is served (the reserve, then the render pool), the list grows by
 *   256 pages a step, and each step is a request and its notice: past
 *   slot 12 on the ring's first lap after 7 grows.
 * v1 (NO_RESERVE) refused the first grow, and that kick hung even with a
 * 3D pending (A = 1): no traffic followed.
 * Each phase prints the driver's counters (pvr_render_fl_counters). The case
 * fails when a slot was read empty or the service stopped; refusals are
 * printed (v4 makes them on purpose). It checks no pixel: its read-out is
 * the counters.
 * ------------------------------------------------------------------------- */
#ifndef CONFIG_GL_TEST_FWCCB_LOSS_SECONDS
#define CONFIG_GL_TEST_FWCCB_LOSS_SECONDS 25
#endif
#define FWL_SIZE  1024u
#define FWL_MAXQ  8192u
#define FWL_PHASE 16u
#define FWL_TILES 32u    /* tiles a side, 32 pixels each */
#define FWL_LPD   8u     /* layers a draw: the frame's draws stay few */
#ifndef CONFIG_GL_TEST_FWCCB_LOSS_MAX_LAYERS
#define CONFIG_GL_TEST_FWCCB_LOSS_MAX_LAYERS 1024
#endif
/* layers a frame at most (v8 on the AM62P: 30, where the held runs of v7
 * survived 47-53 phases and one more step, 33, hangs a frame alone) */
#define FWL_MAXL  ((uint32_t)CONFIG_GL_TEST_FWCCB_LOSS_MAX_LAYERS)
#ifndef CONFIG_GL_TEST_FWCCB_LOSS_HOLD_GROWS
#define CONFIG_GL_TEST_FWCCB_LOSS_HOLD_GROWS 10
#endif
/* served grows after which L stops rising */
#define FWL_HOLD_GROWS ((uint32_t)CONFIG_GL_TEST_FWCCB_LOSS_HOLD_GROWS)
static struct cuv_vertex fwl_vtx[FWL_MAXQ * 6u];

/* Case 174's ramp (v6): the layers of the next phase, from this one's
 * requests and the served grows so far. Rise by an eighth (at least one)
 * while the phase asked in fewer than three quarters of its frames; hold for
 * good (`*held`) once it asked that often (glmark2's refract: every frame
 * asks and is refused), once the run made FWL_HOLD_REQUESTS requests (v7),
 * after FWL_HOLD_GROWS served grows (v3), or at FWL_MAXL. v5 held at the
 * first refusal, where the following frames fit and nothing asked again; v6
 * rose past the load at which one frame alone exceeds 1280 pages (33 layers)
 * and every run hung at phase 15 (cause 4), after slot 12. */
#define FWL_HOLD_REQUESTS 16u /* past request 7 (slot 12 on lap 1) with margin */
static uint32_t fwl_next(uint32_t n, uint32_t phase_requests, uint32_t requests, uint32_t served, bool *held)
{
	uint32_t step = n / 8u ? n / 8u : 1u;

	if (*held || phase_requests * 4u >= FWL_PHASE * 3u || requests >= FWL_HOLD_REQUESTS ||
	    served >= FWL_HOLD_GROWS || n >= FWL_MAXL) {
		*held = true;
		return n;
	}
	return n + step > FWL_MAXL ? FWL_MAXL : n + step;
}

/* FWL_LPD layers of case 174 in fwl_vtx: a 4x4-pixel quad inside each
 * tile, the same quads again in each layer. */
static void fwl_grid(void)
{
	for (uint32_t l = 1; l < FWL_LPD; l++)
		memcpy(&fwl_vtx[l * FWL_TILES * FWL_TILES * 6u], fwl_vtx,
		       FWL_TILES * FWL_TILES * 6u * sizeof(fwl_vtx[0]));
}

static void fwl_layer(void)
{
	for (uint32_t ty = 0; ty < FWL_TILES; ty++)
		for (uint32_t tx = 0; tx < FWL_TILES; tx++) {
			struct cuv_vertex *v = &fwl_vtx[(ty * FWL_TILES + tx) * 6u];
			float x0 = ((float)tx * 32.0f + 14.0f) * 2.0f / FWL_SIZE - 1.0f, x1 = x0 + 8.0f / FWL_SIZE;
			float y0 = ((float)ty * 32.0f + 14.0f) * 2.0f / FWL_SIZE - 1.0f, y1 = y0 + 8.0f / FWL_SIZE;
			float r = (float)(tx % 7u) / 7.0f, g = (float)(ty % 11u) / 11.0f, b = 0.5f;

			cuv_set(&v[0], x0, y0, 0, r, g, b);
			cuv_set(&v[1], x1, y0, 0, r, g, b);
			cuv_set(&v[2], x0, y1, 0, r, g, b);
			cuv_set(&v[3], x1, y0, 0, r, g, b);
			cuv_set(&v[4], x1, y1, 0, r, g, b);
			cuv_set(&v[5], x0, y1, 0, r, g, b);
		}
}

static int case_fwccbloss(void)
{
	GLuint prog = link_program(gl_test_vs_src, gl_test_fs_src), tex, fbo, vbo;
	uint32_t c0[9], c[9], p[9], n = 8, phase = 0, t0;
	bool held = false;

	if (!prog)
		return -1;
	fwl_layer();
	fwl_grid();
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, FWL_SIZE, FWL_SIZE, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glViewport(0, 0, FWL_SIZE, FWL_SIZE);
	vbo = upload_vbo(fwl_vtx, FWL_LPD * FWL_TILES * FWL_TILES * 6u * sizeof(fwl_vtx[0]));
	glUseProgram(prog);
	cuv_attribs();
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_BLEND);
	if (!check_gl_error("fwccbloss setup"))
		return -1;
	pvr_render_fl_counters(c0);
	memcpy(p, c0, sizeof(p));
	memcpy(c, c0, sizeof(c));
	t0 = now_ms();
	while (now_ms() - t0 < (uint32_t)CONFIG_GL_TEST_FWCCB_LOSS_SECONDS * 1000u) {
		for (uint32_t f = 0; f < FWL_PHASE; f++) {
			glClearColor(0, 0, 0, 1);
			glClear(GL_COLOR_BUFFER_BIT);
			for (uint32_t l = 0; l < n; l += FWL_LPD) {
				uint32_t k = n - l < FWL_LPD ? n - l : FWL_LPD;

				glDrawArrays(GL_TRIANGLES, 0, (GLsizei)(k * FWL_TILES * FWL_TILES * 6u));
			}
			glFlush();
		}
		pvr_render_fl_counters(c);
		pr_info("gl_test: fwccbloss phase %u, %u layers: requests %u (served %u, refused %u), "
			"notices %u, slots read empty %u (filled %u), re-posts %u, stop %u\n",
			phase, n, c[0] - p[0], c[1] - p[1], c[2] - p[2], c[3] - p[3], c[4] - p[4], c[5] - p[5],
			c[6] - p[6], c[7]);
		if (c[7] != 0)
			break;
		n = fwl_next(n, c[0] - p[0], c[0] - c0[0], c[1] - c0[1], &held);
		memcpy(p, c, sizeof(p));
		phase++;
	}
	glFinish();
	pvr_render_fl_counters(c);
	pr_info("gl_test: fwccbloss: %u phases, last %u layers, %u ms: requests %u (served %u, refused %u), "
		"notices %u, FWCCB commands about %u, slots read empty %u (filled %u), re-posts %u, stop %u\n",
		phase, n, now_ms() - t0, c[0] - c0[0], c[1] - c0[1], c[2] - c0[2], c[3] - c0[3],
		(c[0] - c0[0]) + (c[3] - c0[3]), c[4] - c0[4], c[5] - c0[5], c[6] - c0[6], c[7]);
	glDisableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vbo);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glViewport(0, 0, TW, TH);
	/* a loss or a stop is a failure whatever the count */
	if (c[4] != c0[4] || c[7] != 0)
		return -1;
	if ((c[0] - c0[0]) + (c[3] - c0[3]) < 13u) {
		pr_info("gl_test: fwccbloss: NOT EXERCISED: %u FWCCB commands, fewer than glmark2's 13 at the loss\n",
			(c[0] - c0[0]) + (c[3] - c0[3]));
		return -3; /* skipped by name: not a verdict on the queue */
	}
	/* refusals are printed, not a failure: v5 makes them on purpose (the
	 * probe), and v3's would show in the phase lines */
	return 0;
}

/* ---- case 175: a refused grow with no 3D pending (PowerVR) ------------------
 *
 * The AM62P glmark2 cell's hang: a freelist grow the host refuses while no
 * kick's 3D is pending to return pages. The out-of-memory TA then has
 * nothing to wait for and never completes, because this driver queues no
 * partial-render command (driver/docs/pvr-render-pool.md, "Design: the
 * firmware's partial render"). d320c8a5 met it in glmark2 with one kick in
 * flight. This case is that, alone: one kick in flight (glFinish after every
 * frame), frames of N full-screen quads into a 1024x1024 target, N up by a
 * quarter every frame until a frame asks for a grow. Built with
 * CONFIG_SAFEUI_GPU_PVR_FL_PROBE_NO_RESERVE that grow is refused at A = 0.
 * - Today: the frame's kick never completes and the render service stops at
 *   10 s (cause 4, "a kick that did not complete in the stop's time"), in
 *   every configuration (a842cffd). The record names it. The case then
 *   prints the counters and fails.
 * - With the partial render: the frame completes with "partial renders"
 *   >= 1, and the case passes.
 * ------------------------------------------------------------------------- */
static int case_refusehang(void)
{
	GLuint prog = link_program(gl_test_vs_src, gl_test_fs_src), tex, fbo, vbo;
	uint32_t c0[9], c[9], n = 32, frames = 0;
	bool asked = false;

	if (!prog)
		return -1;
	for (uint32_t q = 0; q < FWL_MAXQ; q++) {
		struct cuv_vertex *v = &fwl_vtx[q * 6u];
		float r = (float)(q % 7u) / 7.0f, g = (float)(q % 11u) / 11.0f, b = (float)(q % 13u) / 13.0f;

		cuv_set(&v[0], -1, -1, 0, r, g, b);
		cuv_set(&v[1], 1, -1, 0, r, g, b);
		cuv_set(&v[2], -1, 1, 0, r, g, b);
		cuv_set(&v[3], 1, -1, 0, r, g, b);
		cuv_set(&v[4], 1, 1, 0, r, g, b);
		cuv_set(&v[5], -1, 1, 0, r, g, b);
	}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, FWL_SIZE, FWL_SIZE, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glViewport(0, 0, FWL_SIZE, FWL_SIZE);
	vbo = upload_vbo(fwl_vtx, sizeof(fwl_vtx));
	glUseProgram(prog);
	cuv_attribs();
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_BLEND);
	if (!check_gl_error("refusehang setup"))
		return -1;
	pvr_render_fl_counters(c0);
	memcpy(c, c0, sizeof(c));
	while (!asked && n <= FWL_MAXQ && c[7] == 0) {
		glClearColor(0, 0, 0, 1);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, (GLsizei)(n * 6u));
		glFinish();
		frames++;
		pvr_render_fl_counters(c);
		asked = c[0] != c0[0];
		if (!asked)
			n += n / 4u;
	}
	pr_info("gl_test: refusehang: %u frames, the last of %u quads %s: requests %u (served %u, refused %u), "
		"notices %u, re-posts %u, stop %u, partial renders %u\n",
		frames, n, asked ? "asked for a grow" : "never asked", c[0] - c0[0], c[1] - c0[1], c[2] - c0[2],
		c[3] - c0[3], c[6] - c0[6], c[7], c[8] - c0[8]);
	glDisableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vbo);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	glViewport(0, 0, TW, TH);
	if (!asked) {
		pr_info("gl_test: refusehang: NOT EXERCISED: no frame up to %u quads asked for a grow\n", FWL_MAXQ);
		return -1;
	}
	return c[7] == 0 && c[8] != c0[8] ? 0 : -1;
}
#endif

/* ---- case 103: a colour-masked draw in front of an earlier one --------------
 *
 * A red quad (z 0.5), then a green one in front of it (z 0) drawn with
 * glColorMask(0, 1, 0, 1), depth test LESS and depth writes on for both. GL
 * keeps the red channel of the pixel under the green quad -- red from A,
 * green from B: yellow. On the PowerVR the colour mask is lowered into the
 * fragment shader (it reads the pixel back for the channels left off,
 * `uses_fbfetch`); a depth-writing draw without blending used to run OPAQUE,
 * and hidden surface removal then never shaded A under B, so B read back the
 * clear's red (0) and the pixel was green. With pvr_debug_masked_clear on
 * (CONFIG_GL_TEST_PVR_MASKED_CLEAR / GL_TEST_PVR_MASKED_CLEAR=1) the backend
 * runs a shader that reads the pixel TRANSLUCENT, as Mesa's pvr does; with it
 * off -- the default until this case and the regression set have run -- the
 * PowerVR fails this case by design.
 *
 *   under B:  0xff00ffff  (R from A, G from B)
 *   A only:   0xff0000ff
 *
 * Online-path only if the offline table has no colour-mask variant of the
 * colour shader.
 * ------------------------------------------------------------------------- */
static int case_maskdepth(void)
{
	struct cuv_vertex v[12];
	GLuint prog;
	bool ok = true;

	prog = link_program(gl_test_vs_src, gl_test_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	/* A: red, the whole target, z 0.5 */
	cuv_set(&v[0], -1.0f, -1.0f, 0.5f, 1, 0, 0);
	cuv_set(&v[1], 1.0f, -1.0f, 0.5f, 1, 0, 0);
	cuv_set(&v[2], -1.0f, 1.0f, 0.5f, 1, 0, 0);
	cuv_set(&v[3], 1.0f, -1.0f, 0.5f, 1, 0, 0);
	cuv_set(&v[4], 1.0f, 1.0f, 0.5f, 1, 0, 0);
	cuv_set(&v[5], -1.0f, 1.0f, 0.5f, 1, 0, 0);
	/* B: green, the middle half, z 0 (in front) */
	cuv_set(&v[6], -0.5f, -0.5f, 0.0f, 0, 1, 0);
	cuv_set(&v[7], 0.5f, -0.5f, 0.0f, 0, 1, 0);
	cuv_set(&v[8], -0.5f, 0.5f, 0.0f, 0, 1, 0);
	cuv_set(&v[9], 0.5f, -0.5f, 0.0f, 0, 1, 0);
	cuv_set(&v[10], 0.5f, 0.5f, 0.0f, 0, 1, 0);
	cuv_set(&v[11], -0.5f, 0.5f, 0.0f, 0, 1, 0);
	upload_vbo(v, sizeof(v));
	cuv_attribs();

	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glDepthMask(GL_TRUE);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glColorMask(GL_FALSE, GL_TRUE, GL_FALSE, GL_TRUE);
	glDrawArrays(GL_TRIANGLES, 6, 6);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glFinish();
	glDisable(GL_DEPTH_TEST);
	if (!check_gl_error("maskdepth"))
		return -1;
	read_topdown();
	ok &= expect_px("maskdepth: under B, R from A and G from B", TW / 2, TH / 2, 0xff00ffffu, 3);
	ok &= expect_px("maskdepth: A only", TW / 8, TH / 8, 0xff0000ffu, 3);
	return ok && check_gl_error("maskdepth teardown") ? 0 : -1;
}

static int run_case(int c, int pretransform, int debug)
{
	int rc;
#ifdef GL_TEST_PVR
	/* A fence wait that timed out during the case means its read-backs saw
	 * whatever the GPU had finished, not the frame the case asked for: that
	 * is its own outcome, TIMEOUT, never a pixel verdict (case 13's check 9
	 * read the previous kick's picture after two 20 s timeouts). */
	const uint32_t timeouts0 = pvr_backend_wait_timeouts();
#endif
	/* A shader set that has no form of some case's shaders skips it BY
	 * NAME. The ES 1.00 set has no twin for the fourteen #version 300 es
	 * sources, because an ES 2.0 application cannot write one -- that is a
	 * property of the dialect, not a gap in the compiler, so those cases
	 * must never read as a FAIL and must never pass silently either.
	 *
	 * Here as well as in the sweep loop so that a single-case run says why
	 * too; the loop filters first, so this is reached only by -c. */
	const char *skip = shader_skip_reason(c);

	if (skip) {
		pr_info("gl_test: case %d %s: SKIP (%s)\n", c,
			(c >= 0 && c < (int)(sizeof(case_names) / sizeof(case_names[0])))
				? case_names[c] : "?", skip);
		/* -3, on the same reasoning as -2 below: "there is no such
		 * case" is not "the case failed", and neither is a skip. */
		return -3;
	}

	/* So a caller watching from another thread can say which case the
	 * suite was in when it stopped; on a board with no console that is
	 * the difference between "it hung" and "it hung in case 13". */
	int prev = gl_test_current_case;

	gl_test_current_case = c;
	gl_test_check_no = 0;
	gl_test_first_bad_check = -1;
	gl_test_first_bad_got = 0;
	gl_test_first_bad_want = 0;
	reset_state(prev);
	switch (c) {
	case 0: rc = case_triangle(pretransform, debug); break;
	case 1: rc = case_uniform(); break;
	case 2: rc = case_texture(); break;
	case 3: rc = case_depth(); break;
	case 4: rc = case_cull(); break;
	case 5: rc = case_scissor(); break;
	case 6: rc = case_fbo(); break;
	case 8: rc = case_blend(); break;
	case 9: rc = case_stencil(); break;
	case 10: rc = case_prims(); break;
	case 11: rc = case_persist(); break;
	case 12: rc = case_stencil2(); break;
	case 13: rc = case_zls(); break;
	case 14: rc = case_api(); break;
	case 15: rc = case_egl(); break;
	case 16:
		rc = case_msaa();
		if (rc == -3)
			return -3; /* skipped by name: the 1x half passed, no 4x config in this build */
		break;
	case 17: rc = case_ctx2(); break;
	case 18: rc = case_image(); break;
	case 19: rc = case_es100(); break;
	case 20: rc = case_etc(); break;
	case 126: rc = case_etcmip(); break;
	case 21: rc = case_rgb565(); break;
	case 172: rc = case_swapzs(); break;
	case 173: rc = case_splitzs(); break;
#ifdef GL_TEST_PVR
	case 174:
		rc = case_fwccbloss();
		if (rc == -3)
			return -3; /* skipped by name: fewer than 13 FWCCB commands */
		break;
	case 175: rc = case_refusehang(); break;
#endif
	case 176: rc = case_mapbuf(); break;
	case 22: rc = case_vao(); break;
	case 23: rc = case_winding(); break;
	case 24: rc = case_compact(); break;
	case 25: rc = case_stress(); break;
	case 26: rc = case_heavy(); break;
	case 27: rc = case_fvgrepro(); break;
	case 28: rc = case_fvgvs(); break;
	case 29: rc = case_fvgft(); break;
	case 30: rc = case_fvgin(); break;
	case 31: rc = case_fvgcov(); break;
	case 32: rc = case_fvgdis(); break;
	case 33: rc = case_fvgdis2(); break;
	case 34: rc = case_unishare(); break;
	case 35: rc = case_tex1d(); break;
	case 36: rc = case_fvgconic(); break;
	case 37: rc = case_fvgstroke(); break;
	case 38: rc = case_fvgstencil(); break;
	case 39: rc = case_perf(); break;
	case 41: rc = case_es3cube(); break;
	case 42: rc = case_es3array(); break;
	case 43: rc = case_es3vid(); break;
	case 46: rc = case_es3size(); break;
	case 47: rc = case_es3inst(); break;
	case 141: rc = case_es3instdiv(); break;
	case 51: rc = case_es3heavy(); break;
	case 52: rc = case_es3split(); break;
	case 54: rc = case_es3chain(); break;
	case 55: rc = case_es3mrt(); break;
	case 56: rc = case_manylines(); break;
	case 57: rc = case_es3vid_indexed(); break;
	case 48: rc = case_es3vol(); break;
	case 49: rc = case_es3facing(); break;
	case 45: rc = case_es3dyn(); break;
	case 44: rc = case_es3hdr(); break;
	case 40: rc = case_qtstrip(); break;
	case 53: rc = case_zprobe(); break;
	case 58: rc = case_clearscis(); break;
	case 59: rc = case_clearmask(); break;
	case 60: rc = case_wrapmode(); break;
	case 61: rc = case_texincomp(); break;
	case 62: rc = case_clipz(); break;
	case 63: rc = case_matattr(); break;
	case 64: rc = case_bgra(); break;
	case 65: rc = case_polyunit(); break;
	case 66: rc = case_clearvp(); break;
	case 67: rc = case_fbods(); break;
	case 68: rc = case_armselect(); break;
	case 69: rc = case_ioswap(); break;
	case 70: rc = case_loopcarry(); break;
	case 71: rc = case_deadloop(); break;
	case 72: rc = case_intlit(); break;
	case 73: rc = case_branchlane(); break;
	case 74: rc = case_ifoutput(); break;
	case 75: rc = case_immneg(); break;
	case 76: rc = case_boolany(); break;
	case 77: rc = case_transc(); break;
	case 78: rc = case_matvary(); break;
	case 79: rc = case_idiv(); break;
	case 80: rc = case_matdyn(); break;
	case 81: rc = case_branchquad(); break;
	case 90: rc = case_churn(); break;
	case 82: rc = case_fragcentre(); break;
	case 83: rc = case_pointcoord(); break;
	case 84: rc = case_fragcoord(); break;
	case 85: rc = case_cubetail(); break;
	case 86: rc = case_anyall(); break;
	case 87: rc = case_databreak(); break;
	case 88: rc = case_rsflip(); break;
	case 89: rc = case_flatlast(); break;
	case 91: rc = case_hdrblend(); break;
	case 97:
		rc = case_varyings15();
		if (rc == -3)
			return -3; /* skipped by name, above: neither a pass nor a fail */
		break;
	case 98: rc = case_uniloop(); break;
	case 100: rc = case_idxtex(); break;
	case 101: rc = case_cubebias(); break;
	case 102: rc = case_maskclear(); break;
	case 103: rc = case_maskdepth(); break;
	case 104: rc = case_divloop(); break;
	case 105: rc = case_pointcoordvary(); break;
	case 107: rc = case_clearload(); break;
	case 108: rc = case_uniidx(); break;
	/* dFdx / dFdy read back: plain GL, every GPU (the GC7000's dsx/dsy and
	 * the PowerVR's derivatives are measured here as the V3D's are). */
	case 138: rc = case_deriv(); break;
	/* Plain GL, written for the V3D, run on every GPU: a uniform branch
	 * with both arms read back, look-ups inside uniform branches, and
	 * glSampleCoverage, which skips by name where the build offers no 4x
	 * config (the V3D's v3d_debug_msaa, the GC7000's gc_debug_msaa off).
	 * Their dispatch sat in the V3D block below, so the other GPUs compiled
	 * the three functions and never called them (-Wunused-function). */
	case 142: rc = case_unibranch(); break;
	case 148: rc = case_texbranch(); break;
	case 145:
		rc = case_samplecov();
		if (rc == -3)
			return -3; /* skipped by name: no 4x config in this build */
		break;
#ifdef GL_TEST_V3D
	case 109: rc = case_texlinear(); break;
	case 110: rc = case_psizvpm(); break;
	case 111: rc = case_points(); break;
	case 115: rc = case_texcube(); break;
	case 118: rc = case_texmip(); break;
	case 133: rc = case_texcubemip(); break;
	case 134: rc = case_texcubethin(); break;
	case 136: rc = case_psizmax(); break;
	case 135: rc = case_texnarrow(); break;
	case 137: rc = case_psizmin(); break;
	case 140: rc = case_tfuprobe(); break;
	case 124: rc = case_binovf(); break;
	case 125: rc = case_texsubfbo(); break;
	case 129: rc = case_vstexlod(); break;
	case 130: rc = case_allfeat3(); break;
	case 131: rc = case_allfeat1(); break;
#endif
#ifdef GL_TEST_VIVANTE
	/* dEQP texture.vertex.2d fails 45 of 45 on the i.MX8MP too, and its
	 * GC7000 stream is Mesa's for that shape (golden modes 26, 27): the
	 * passes' first bad check says which ingredient on silicon */
	case 129: rc = case_vstexlod(); break;
#endif
	case 132: rc = case_narrowla(); break;
	/* every GPU: the case skips itself by name on the GC7000 */
	case 149:
		rc = case_vsunits();
		if (rc == -3)
			return -3; /* skipped by name: the GC7000's vertex stage has 8 units */
		break;
	/* every GPU; not in the default set (a probe, see case_shprobe) */
	case 150: rc = case_shprobe(); break;
	case 162: rc = case_dskeep(); break;
	case 167: rc = case_stcycle(); break;
	case 168: rc = case_stlayers(); break;
	/* every GPU; not in the default set (a probe, see case_fshprobe) */
	case 166: rc = case_fshprobe(); break;
#if defined(GL_TEST_V3D) || !defined(GL_TEST_VIVANTE)
	case 123: rc = case_zsapart(); break;
#endif
	case 112: rc = case_affectand(); break;
	case 113: rc = case_cubemap(); break;
	case 116: rc = case_uniidx_vs(1); break;
	case 117: rc = case_uniidx_vs(2); break;
	case 114: rc = case_cubelod(); break;
	case 119: rc = case_pbufup(); break;
	case 120: rc = case_uniidx_tex(0); break;
	case 121: rc = case_uniidx_tex(1); break;
	case 122: rc = case_uniidx_tex(2); break;
	case 127: rc = case_lines(); break;
	case 128: rc = case_uniidx_varying(); break;
	case 106: rc = case_mrt3(); break;
	case 164: rc = case_bufdel(); break;
	case 165:
		rc = case_ctxclear();
		if (rc == -3)
			return -3; /* skipped by name: no free context slot */
		break;
	case 171: rc = case_viewwin(); break;
	case 169:
		rc = case_dstex();
		if (rc == -3)
			return -3; /* skipped by name: the V3D, or an ES 2.0 library without the extensions */
		break;
	case 180: rc = case_teardown(); break;
	case 181: rc = case_fusemad(); break;
	case 182: rc = case_fsat(); break;
	case 183: rc = case_unipad(); break;
	case 184: rc = case_depchain(); break;
	case 185: rc = case_kindcost(); break;
	case 186: rc = case_fcsplit(); break;
	case 187: rc = case_predarm(); break;
	case 188: rc = case_predif(); break;
	case 189: rc = case_mulone(); break;
	case 190: rc = case_blendonce(); break;
	case 191: rc = case_varyfma(); break;
	case 192: rc = case_predlive(); break;
	case 193: rc = case_offarm(); break;
	case 194: rc = case_offarmn(); break;
	case 195: rc = case_ifbranch(); break;
	case 196: rc = case_occl(); break;
	case 197: rc = case_vtxgrid(); break;
	case 179:
		rc = case_retile();
		if (rc == -3)
			return -3; /* skipped by name: a GPU with no depth-only target, or no extension */
		break;
	case 177:
		rc = case_zonly();
		if (rc == -3)
			return -3; /* skipped by name: a GPU with no depth-only target, the V3D, or no extension */
		break;
	case 178:
		rc = case_heldfree();
		if (rc == -3)
			return -3; /* skipped by name: as case 177 */
		break;
	case 163:
		rc = case_drawbuf0();
		if (rc == -3)
			return -3; /* skipped by name: an ES 2.0 library, or one draw buffer */
		break;
	/* -2, not -1: "there is no such case" is not "the case failed", and a
	 * caller sweeping a range of numbers has to be able to tell them
	 * apart. The numbering has gaps (7 is the regression set, 50 is
	 * unused), and counting a gap as a failure invents one. */
	default: pr_err("gl_test: no case %d\n", c); return -2;
	}
	{
		const char *verdict = rc == 0 ? "PASS" : "FAIL";

#ifdef GL_TEST_PVR
		const uint32_t timeouts = pvr_backend_wait_timeouts() - timeouts0;

		if (timeouts) {
			verdict = "TIMEOUT";
			rc = -4;
			pr_info("gl_test: case %d: %u fence wait(s) timed out; its checks are not a verdict\n",
				c, (unsigned)timeouts);
		}
#endif
		pr_info("gl_test: case %d %s: %s\n", c,
			(c >= 0 && c < (int)(sizeof(case_names) / sizeof(case_names[0]))) ? case_names[c] : "?",
			verdict);
	}
	return rc;
}

/* ---- panel: draw on the LVDS scan-out buffer ------------------------------- */

/* The Linux harness draws into the framebuffer tidss scans out (XRGB8888,
 * pitch 4096, see dri debugfs); a Zephyr build may override the three. */
#ifndef PANEL_W
#define PANEL_W 1024u
#endif
#ifndef PANEL_H
#define PANEL_H 600u
#endif
#ifndef PANEL_PHYS
#define PANEL_PHYS 0xE2900000ull
#endif

/* The scan-out buffer as an EGL window surface: eglInitialize (the OSAL and
 * backend are up already; both accept the repeat), one config, a window
 * surface over the panel's physical pages, an ES2 context. */
static EGLDisplay s_egl_dpy;
static EGLSurface s_egl_surf;

static int panel_context(int samples)
{
	static struct gles_native_window win;
	EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
		EGL_DEPTH_SIZE, 16, EGL_STENCIL_SIZE, 8,
		EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_SAMPLES, samples,
		EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLConfig cfg;
	EGLContext ctx;
	EGLint n = 0, major = 0, minor = 0;

	win.phys = PANEL_PHYS;
	win.width = PANEL_W;
	win.height = PANEL_H;
	win.stride_bytes = PANEL_W * 4;
	win.format = GLES_NATIVE_FORMAT_XRGB8888;

	s_egl_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_egl_dpy, &major, &minor) ||
	    !eglChooseConfig(s_egl_dpy, cfg_attribs, &cfg, 1, &n) || n != 1) {
		pr_err("gl_test: EGL init/config failed (0x%x)\n", eglGetError());
		return -1;
	}
	s_egl_surf = eglCreateWindowSurface(s_egl_dpy, cfg, &win, NULL);
	ctx = eglCreateContext(s_egl_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (s_egl_surf == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT ||
	    !eglMakeCurrent(s_egl_dpy, s_egl_surf, s_egl_surf, ctx)) {
		pr_err("gl_test: EGL surface/context failed (0x%x)\n", eglGetError());
		return -1;
	}
	pr_info("gl_test: panel: EGL %d.%d, %ux%u XRGB8888 window at phys 0x%llx, %d samples\n",
		major, minor, PANEL_W, PANEL_H, PANEL_PHYS, samples ? samples : 1);
	return 0;
}

/* A scene for the camera: dark blue background, the Gouraud triangle in the
 * middle, a magnified checker texture at the top-left, a half-transparent
 * white square (blend) over the triangle's bottom, a yellow line loop frame. */
static int panel_demo(void)
{
	struct cuv_vertex v[16];
	GLuint tri_prog, tex_prog;
	static const uint8_t checker[4 * 4 * 4] = {
		255,0,0,255, 0,255,0,255, 255,0,0,255, 0,255,0,255,
		0,255,0,255, 255,0,0,255, 0,255,0,255, 255,0,0,255,
		255,0,0,255, 0,255,0,255, 255,0,0,255, 0,255,0,255,
		0,255,0,255, 255,0,0,255, 0,255,0,255, 255,0,0,255,
	};
	GLuint tex = 0;

	tri_prog = link_program(gl_test_vs_src, gl_test_fs_src);
	tex_prog = link_program(gl_test_uv_vs_src, gl_test_texture_fs_src);
	if (!tri_prog || !tex_prog)
		return -1;

	glViewport(0, 0, PANEL_W, PANEL_H);
	glClearColor(0.05f, 0.05f, 0.25f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);

	/* the triangle */
	glUseProgram(tri_prog);
	cuv_set(&v[0], 0.0f, 0.8f, 0.0f, 1, 0, 0);
	cuv_set(&v[1], -0.6f, -0.7f, 0.0f, 0, 1, 0);
	cuv_set(&v[2], 0.6f, -0.7f, 0.0f, 0, 0, 1);
	/* a translucent white square over the bottom of the triangle */
	cuv_set(&v[3], -0.3f, -0.9f, 0.0f, 1, 1, 1);
	cuv_set(&v[4], 0.3f, -0.9f, 0.0f, 1, 1, 1);
	cuv_set(&v[5], 0.3f, -0.3f, 0.0f, 1, 1, 1);
	cuv_set(&v[6], -0.3f, -0.9f, 0.0f, 1, 1, 1);
	cuv_set(&v[7], 0.3f, -0.3f, 0.0f, 1, 1, 1);
	cuv_set(&v[8], -0.3f, -0.3f, 0.0f, 1, 1, 1);
	for (int i = 3; i < 9; i++)
		v[i].color[3] = 0.5f;
	/* a yellow frame just inside the panel edge */
	cuv_set(&v[9], -0.98f, -0.97f, 0.0f, 1, 1, 0);
	cuv_set(&v[10], 0.98f, -0.97f, 0.0f, 1, 1, 0);
	cuv_set(&v[11], 0.98f, 0.97f, 0.0f, 1, 1, 0);
	cuv_set(&v[12], -0.98f, 0.97f, 0.0f, 1, 1, 0);
	upload_vbo(v, sizeof(v));
	cuv_attribs();
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glEnable(GL_BLEND);
	glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
	glDrawArrays(GL_TRIANGLES, 3, 6);
	glDisable(GL_BLEND);
	glLineWidth(6.0f);
	glDrawArrays(GL_LINE_LOOP, 9, 4);
	glLineWidth(1.0f);

	/* the checker texture, magnified into the top-left corner */
	glUseProgram(tex_prog);
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, checker);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	glDisableVertexAttribArray(2);
	puv_quad_setup();
	glViewport(40, PANEL_H - 40 - 200, 200, 200);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glViewport(0, 0, PANEL_W, PANEL_H);
	glFinish();
	if (!check_gl_error("panel demo"))
		return -1;
	pr_info("gl_test: panel demo drawn\n");
	return 0;
}

/* ---- panel: a spinning 3D cube ------------------------------------------ */

/* sin/cos for the animation (no libm in the kernel): range-reduce to
 * [-pi, pi] and sum the Taylor series to x^11 (error < 1e-6). */
static float sin_approx(float x)
{
	const float pi = 3.14159265f, two_pi = 6.28318531f;
	float x2, term, sum;

	while (x > pi) x -= two_pi;
	while (x < -pi) x += two_pi;
	x2 = x * x;
	term = x;
	sum = x;
	for (int i = 1; i <= 5; i++) {
		term *= -x2 / (float)((2 * i) * (2 * i + 1));
		sum += term;
	}
	return sum;
}

static float cos_approx(float x)
{
	return sin_approx(x + 1.57079633f);
}

/* column-major 4x4: m[col * 4 + row] */
static void mat4_mul(float *o, const float *a, const float *b)
{
	float t[16];

	for (int c = 0; c < 4; c++)
		for (int r = 0; r < 4; r++)
			t[c * 4 + r] = a[0 * 4 + r] * b[c * 4 + 0] + a[1 * 4 + r] * b[c * 4 + 1] +
				       a[2 * 4 + r] * b[c * 4 + 2] + a[3 * 4 + r] * b[c * 4 + 3];
	memcpy(o, t, sizeof(t));
}

static void mat4_identity(float *m)
{
	memset(m, 0, 16 * sizeof(float));
	m[0] = m[5] = m[10] = m[15] = 1.0f;
}

/* OpenGL perspective (fov in radians, y up, z into -w). */
static void mat4_perspective(float *m, float fov, float aspect, float n, float f)
{
	float t = 1.0f / (sin_approx(fov * 0.5f) / cos_approx(fov * 0.5f));

	memset(m, 0, 16 * sizeof(float));
	m[0] = t / aspect;
	m[5] = t;
	m[10] = (f + n) / (n - f);
	m[11] = -1.0f;
	m[14] = 2.0f * f * n / (n - f);
}

static void mat4_rotate_xy(float *m, float ax, float ay)
{
	float rx[16], ry[16], cx = cos_approx(ax), sx = sin_approx(ax), cy = cos_approx(ay), sy = sin_approx(ay);

	mat4_identity(rx);
	rx[5] = cx; rx[6] = sx; rx[9] = -sx; rx[10] = cx;
	mat4_identity(ry);
	ry[0] = cy; ry[2] = -sy; ry[8] = sy; ry[10] = cy;
	mat4_mul(m, ry, rx);
}

/* Six faces, two triangles each, a flat colour per face (the sign of the
 * normal picks the shade, like one directional light). */
static void cube_vertices(struct pc_vertex *v)
{
	static const float p[8][3] = {
		{ -1, -1, -1 }, { 1, -1, -1 }, { 1, 1, -1 }, { -1, 1, -1 },
		{ -1, -1, 1 }, { 1, -1, 1 }, { 1, 1, 1 }, { -1, 1, 1 },
	};
	/* counter-clockwise seen from outside */
	static const int faces[6][4] = {
		{ 4, 5, 6, 7 },   /* +z front  */
		{ 1, 0, 3, 2 },   /* -z back   */
		{ 5, 1, 2, 6 },   /* +x right  */
		{ 0, 4, 7, 3 },   /* -x left   */
		{ 7, 6, 2, 3 },   /* +y top    */
		{ 0, 1, 5, 4 },   /* -y bottom */
	};
	static const float col[6][3] = {
		{ 0.95f, 0.25f, 0.2f }, { 0.5f, 0.12f, 0.1f },
		{ 0.2f, 0.85f, 0.3f }, { 0.1f, 0.4f, 0.15f },
		{ 0.25f, 0.45f, 1.0f }, { 0.12f, 0.2f, 0.5f },
	};
	static const int tri[6] = { 0, 1, 2, 0, 2, 3 };

	for (int f = 0; f < 6; f++) {
		for (int i = 0; i < 6; i++) {
			struct pc_vertex *o = &v[f * 6 + i];
			const float *q = p[faces[f][tri[i]]];

			o->pos[0] = q[0]; o->pos[1] = q[1]; o->pos[2] = q[2]; o->pos[3] = 1.0f;
			o->color[0] = col[f][0]; o->color[1] = col[f][1]; o->color[2] = col[f][2];
			o->color[3] = 1.0f;
		}
	}
}

#define CUBE_FRAMES 120

static int panel_cube(void)
{
	static struct pc_vertex v[36];
	GLuint prog;
	GLint loc;
	float proj[16], rot[16], view[16], mvp[16];

	prog = link_program(gl_test_mvp_color_vs_src, gl_test_interp_fs_src);
	if (!prog)
		return -1;
	glUseProgram(prog);
	loc = glGetUniformLocation(prog, "u_mvp");
	if (loc < 0) {
		pr_err("gl_test: u_mvp not found\n");
		return -1;
	}
	cube_vertices(v);
	upload_vbo(v, sizeof(v));
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct pc_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	glViewport(0, 0, PANEL_W, PANEL_H);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_LESS);
	glEnable(GL_CULL_FACE);
	glCullFace(GL_BACK);
	glFrontFace(GL_CCW);
	glClearColor(0.05f, 0.05f, 0.12f, 1.0f);
	mat4_perspective(proj, 0.8f, (float)PANEL_W / (float)PANEL_H, 1.0f, 20.0f);
	mat4_identity(view);
	view[14] = -5.0f;   /* the cube 5 units in front of the camera */

	/* CUBE_FRAMES frames of rotation; each eglSwapBuffers waits for the
	 * render, so this is as fast as the GPU goes */
	for (int frame = 0; frame < CUBE_FRAMES; frame++) {
		float a = (float)frame * 0.05f;

		mat4_rotate_xy(rot, a * 0.7f, a);
		mat4_mul(mvp, view, rot);
		mat4_mul(mvp, proj, mvp);
		glUniformMatrix4fv(loc, 1, GL_FALSE, mvp);
		glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 36);
		if (!eglSwapBuffers(s_egl_dpy, s_egl_surf)) {
			pr_err("gl_test: eglSwapBuffers failed (0x%x) at frame %d\n", eglGetError(), frame);
			return -1;
		}
	}
	if (!check_gl_error("cube"))
		return -1;
	pr_info("gl_test: cube: %d frames drawn\n", CUBE_FRAMES);
	return 0;
}

/*
 * What the regression set found, for a caller with no console: how many of
 * its cases passed, which case failed first and where it sits in the order.
 * A board that reports through the time it takes to come back has room for
 * one number at a time, and these are the three worth having.
 */
unsigned int gl_test_passed, gl_test_total;
int gl_test_first_fail = -1, gl_test_first_fail_at = -1;

/* A caller-supplied list of cases to run in one process, in the given order.
 *
 * It exists because the suite's two modes are not interchangeable and the
 * difference is where a defect hides: `all` gives each case its own process,
 * while the regression set walks its cases on ONE context, so a case that
 * corrupts state only fails in the second. Bisecting such a case from the
 * outside is impossible -- separate invocations are separate processes -- so
 * the walker has to be reachable with an arbitrary list.
 *
 * Static storage rather than an argument so that gl_test_run keeps the
 * signature the Zephyr sample and the kernel harness already call it with.
 *
 * GL_TEST_LIST is negative because every real case number is not; 7 is taken
 * by the regression set. */
#define GL_TEST_LIST (-1)
#define GL_TEST_LIST_MAX 256

static int s_list[GL_TEST_LIST_MAX];
static unsigned s_list_n;

/* Store the list and return the `gl_case` value that walks it, writing the
 * number actually stored to *stored (the list is bounded). Returning the
 * sentinel rather than exporting it keeps the caller from spelling either
 * constant itself, so the two cannot drift apart. */
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored)
{
	s_list_n = n > GL_TEST_LIST_MAX ? GL_TEST_LIST_MAX : n;
	for (unsigned i = 0; i < s_list_n; i++) {
		s_list[i] = cases[i];
	}
	if (stored) {
		*stored = s_list_n;
	}
	return GL_TEST_LIST;
}

/* Told each case's result as the walker finishes it: rc as run_case returns
 * it (0 pass, -1 fail, -2 no such case, -3 skipped by the shader set) and -3
 * for a case the walker skipped by name. A harness that can only report at
 * the end of a run (the i.MX8MP's SNVS record) overrides it to keep a
 * running tally that survives a hang half way through. */
__attribute__((weak)) void gl_test_case_result(int c, int rc)
{
	(void)c;
	(void)rc;
}

int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce,
		unsigned int term0, int panel)
{
	struct gl_context *ctx;
	int rc;

	pr_info("gl_test: start (case=%d pretransform=%d ppp_ctrl=0x%x debug=%d no_coalesce=%d)\n",
		gl_case, pretransform, ppp_ctrl, debug, no_coalesce);
#ifdef GL_TEST_VIVANTE
	/* No backend options on this GPU, and the OSAL takes no argument: it
	 * finds the vivante,gc device itself. */
	(void)ppp_ctrl; (void)no_coalesce; (void)term0; (void)debug; (void)pretransform;
	rc = g_osal->init(NULL);
#elif defined(GL_TEST_V3D)
	/* No backend options either; the OSAL takes the board's clock switch. */
	(void)ppp_ctrl; (void)no_coalesce; (void)term0; (void)debug; (void)pretransform;
	rc = g_osal->init(gl_test_v3d_osal_arg());
#else
	{
	struct osal_zephyr_init oi = { .regs = pvr_regs() };
	void *oi_p = &oi;

	pvr_backend_set_option(PVR_BACKEND_OPT_PRETRANSFORM, pretransform ? 1 : 0);
	pvr_backend_set_option(PVR_BACKEND_OPT_PPP_CTRL, ppp_ctrl);
	pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, debug == 1 ? 1 : 0);
	pvr_backend_set_option(PVR_BACKEND_OPT_DS_DUMP, debug == 2 ? 1 : 0);   /* gl_debug=2 */
	pvr_backend_set_option(PVR_BACKEND_OPT_NO_COALESCE, no_coalesce ? 1 : 0);
	pvr_backend_set_option(PVR_BACKEND_OPT_TERM0, term0);

	rc = g_osal->init(oi_p);
	}
#endif
	if (rc) {
		pr_err("gl_test: OSAL init failed %d\n", rc);
		return rc;
	}
	rc = yakogl_backend_init();
	if (rc) {
		pr_err("gl_test: backend init failed %d\n", rc);
		return rc;
	}
	if (panel) {
#ifdef GL_TEST_PVR
		/* the demo is clip-space geometry through the hardware viewport */
		pvr_backend_set_option(PVR_BACKEND_OPT_PRETRANSFORM, 0);
#endif
		if (panel_context(panel == 2 ? 4 : 0) != 0) {   /* the cube is anti-aliased */
			pr_err("gl_test: panel surface setup failed\n");
			return -1;
		}
		rc = panel == 2 ? panel_cube() : panel_demo();
		if (rc == 0 && !eglSwapBuffers(s_egl_dpy, s_egl_surf)) {
			pr_err("gl_test: eglSwapBuffers failed (0x%x)\n", eglGetError());
			rc = -1;
		}
		pr_info("gl_test: done\n");
		return rc;
	}
	ctx = gl_context_create(TW, TH);
	if (!ctx || !gl_default_surface()) {
		pr_err("gl_test: context/surface creation failed\n");
		return -1;
	}
	s_harness_ctx = ctx;

	if (gl_case == 7 || gl_case == GL_TEST_LIST) {
		int fails = 0;
		unsigned timed_out = 0;
		/* the cases that take over the context pool (17, 18) run last */
		static const int order[] = { 1, 2, 3, 4, 5, 6, 8, 9, 10, 22, 11, 34, 12, 23, 13, 40, 14, 24, 58, 59, 60, 61, 15, 16, 19, 20, 41, 42, 43, 46, 47, 48, 49, 45, 44, 172, 173, 181, 182, 183, 188, 189, 190, 191, 192, 197, 21, 17, 18 };
		/* One walker, two callers: the regression set walks `order`, a
		 * caller-supplied list walks its own. They must stay the same
		 * code, because the whole point of the list is to reproduce
		 * what the set does to a case -- a second loop that drifted
		 * would answer a different question than the one asked. */
		const int *list = (gl_case == 7) ? order : s_list;
		unsigned n = (gl_case == 7) ? (unsigned)(sizeof(order) / sizeof(order[0])) : s_list_n;

		gl_test_total = n;
		gl_test_passed = 0;
		gl_test_first_fail = -1;
		gl_test_first_fail_at = -1;
		for (unsigned i = 0; i < n; i++) {
			const char *skip = shader_skip_reason(list[i]);

			if (skip || case_skipped(list[i])) {
				pr_info("gl_test: case %d skipped%s%s\n", list[i],
					skip ? ": " : "", skip ? skip : "");
				gl_test_total--;
				gl_test_case_result(list[i], -3);
				continue;
			}
			int crc = run_case(list[i], pretransform, debug);

			gl_test_case_result(list[i], crc);
			if (crc == -4)
				timed_out++;
			if (crc != 0) {
				fails++;
				if (gl_test_first_fail < 0) {
					gl_test_first_fail = list[i];
					gl_test_first_fail_at = (int)i;
				}
			} else {
				gl_test_passed++;
			}
			/* an EGL case leaves no context current; the next case
			 * runs on the harness context again. 17, 18 and 21
			 * destroy it to free the pool, so a caller list that
			 * runs a case after one of them gets a new one: without
			 * it, [17, 19] ran case 19 with no context at all. */
			if (!s_harness_ctx) {
				/* A case that took over the pool and failed half-way
				 * (18 returns at its first failed check) leaves its
				 * contexts, surfaces and images alive, the pool full
				 * and its own context current: the re-create failed and
				 * every later case ran on that case's context and
				 * pbuffer. eglTerminate releases every resource of the
				 * display (EGL 1.5 3.2); after a case that cleaned up it
				 * is a no-op (the display is not initialised). */
				EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);

				eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
				eglTerminate(dpy);
				s_harness_ctx = gl_context_create(TW, TH);
				if (!s_harness_ctx || !gl_default_surface())
					pr_err("gl_test: the harness context could not be re-created after case %d\n",
					       list[i]);
			}
			if (s_harness_ctx)
				gl_context_set_current(s_harness_ctx);
		}
		/* what the run cost the shader back end: look-ups the precompiled
		 * shaders answered and compiler runs (0 in an image without the
		 * one or the other), so the console proves the tier */
		pr_info("gl_test: all: %d of %u cases failed (%u passed; %u timed out; first failure case %d at index %d); "
			"precompiled hits %u, compiler runs %u\n",
			fails, gl_test_total, gl_test_passed, timed_out, gl_test_first_fail,
			gl_test_first_fail_at, shc_precompiled_hits ? shc_precompiled_hits() : 0u,
			shc_online_compiler_runs ? shc_online_compiler_runs() : 0u);
		rc = fails ? -1 : 0;
	} else {
		rc = run_case(gl_case, pretransform, debug);
	}
	pr_info("gl_test: done\n");
	return rc;
}
