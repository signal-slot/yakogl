/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * deqp_shim.hpp - the few dEQP helpers the negative_api case bodies call.
 *
 * extract.py copies the bodies out of the conformance suite untouched, so this
 * header has to supply what they reference: the log sink (a no-op here, the
 * runner prints its own lines), expectError, the context-info queries, and the
 * handful of glu:: / tcu:: helpers for compressed formats and shader programs.
 * Nothing here decides a verdict except expectError and checkBooleans.
 */
#ifndef YAKOGL_TESTS_DEQP_SHIM_HPP
#define YAKOGL_TESTS_DEQP_SHIM_HPP

#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <GLES3/gl3.h>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include <algorithm>

typedef GLuint GLhandleARB;

/* ---- the case currently running, and its verdict ------------------------ */

struct DeqpCase {
	const char *group;
	const char *name;
	void (*run)(void);
};

extern const char *g_cur_group, *g_cur_name;
extern int g_cur_fails;          /* expectError/checkBooleans mismatches */
extern int g_cur_checks;
extern bool g_verbose;

const char *deqp_error_name(GLenum e);

/* ---- ApiCase::expectError ----------------------------------------------- */

static inline void expectError(GLenum expected)
{
	GLenum err = glGetError();

	g_cur_checks++;
	if (err != expected) {
		g_cur_fails++;
		if (g_verbose)
			printf("    %s.%s: expected %s, got %s\n", g_cur_group, g_cur_name,
			       deqp_error_name(expected), deqp_error_name(err));
	}
}

static inline void expectError(GLenum expected0, GLenum expected1)
{
	GLenum err = glGetError();

	g_cur_checks++;
	if (err != expected0 && err != expected1) {
		g_cur_fails++;
		if (g_verbose)
			printf("    %s.%s: expected %s or %s, got %s\n", g_cur_group, g_cur_name,
			       deqp_error_name(expected0), deqp_error_name(expected1), deqp_error_name(err));
	}
}

static inline void checkBooleans(GLint value, GLboolean expected)
{
	g_cur_checks++;
	if (value != (GLint)expected) {
		g_cur_fails++;
		if (g_verbose)
			printf("    %s.%s: expected %s\n", g_cur_group, g_cur_name,
			       expected ? "GL_TRUE" : "GL_FALSE");
	}
}

static inline void checkBooleans(GLboolean value, GLboolean expected)
{
	checkBooleans((GLint)value, expected);
}

static inline void getSupportedExtensions(GLenum numSupportedValues, GLenum extension,
					  std::vector<int> &values)
{
	GLint num = 0;

	glGetIntegerv(numSupportedValues, &num);
	if (num == 0)
		return;
	values.resize(num);
	glGetIntegerv(extension, &values[0]);
}

/* ---- the log: the runner prints, so the case's own log goes nowhere ------ */

struct DeqpLogSink {
	template <typename T> DeqpLogSink &operator<<(const T &) { return *this; }
};
extern DeqpLogSink m_log;

namespace tcu
{
struct TestLog {
	struct Tag {};
	static Tag Message, EndMessage, EndSection;
	static Tag Section(const char *, const char *) { return Tag(); }
};

/* tcu::toHex is only ever fed to the log */
template <typename T> static inline int toHex(T v) { return (int)v; }

struct InternalError {};

struct IVec3 {
	int m[3];
	IVec3(int a = 0, int b = 0, int c = 0) { m[0] = a; m[1] = b; m[2] = c; }
	int x() const { return m[0]; }
	int y() const { return m[1]; }
	int z() const { return m[2]; }
	int operator[](int i) const { return m[i]; }
};

enum CubeFace { CUBEFACE_NEGATIVE_X = 0, CUBEFACE_POSITIVE_X, CUBEFACE_NEGATIVE_Y,
		CUBEFACE_POSITIVE_Y, CUBEFACE_NEGATIVE_Z, CUBEFACE_POSITIVE_Z, CUBEFACE_LAST };

/* The compressed formats this implementation can advertise. A format we do not
 * know throws, which is what the case bodies catch (tcu::InternalError). */
struct CompressedTexFormat {
	GLenum gl;
	int block_bytes;
};

static inline int getBlockSize(const CompressedTexFormat &f) { return f.block_bytes; }
static inline IVec3 getBlockPixelSize(const CompressedTexFormat &) { return IVec3(4, 4, 1); }
} // namespace tcu

namespace glu
{
static inline tcu::CompressedTexFormat mapGLCompressedTexFormat(GLenum fmt)
{
	tcu::CompressedTexFormat f = { fmt, 0 };

	switch (fmt) {
	case 0x8D64:                 /* GL_ETC1_RGB8_OES */
	case 0x9274:                 /* GL_COMPRESSED_RGB8_ETC2 */
	case 0x9275:                 /* GL_COMPRESSED_SRGB8_ETC2 */
		f.block_bytes = 8; break;
	case 0x9278:                 /* GL_COMPRESSED_RGBA8_ETC2_EAC */
	case 0x9279:                 /* GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC */
		f.block_bytes = 16; break;
	default:
		throw tcu::InternalError();
	}
	return f;
}

/* The context info the cases query. Every answer comes from the GL itself. */
class ContextInfo
{
public:
	int getInt(GLenum pname) const
	{
		GLint v = 0;

		glGetIntegerv(pname, &v);
		glGetError();          /* a query the implementation rejects is not a case failure */
		return v;
	}
	bool isExtensionSupported(const char *name) const
	{
		const char *ext = (const char *)glGetString(GL_EXTENSIONS);

		return ext && strstr(ext, name) != NULL;
	}
	bool isES3Compatible() const
	{
		const char *ver = (const char *)glGetString(GL_VERSION);

		return ver && strncmp(ver, "OpenGL ES 2.0", 13) != 0;
	}
};

/* A linked program the shader cases need a handle to. The sources are the
 * suite's own; a build that cannot compile them still yields a program object,
 * which is all the negative cases dereference. */
struct ProgramSources {
	std::string vert, frag;
};

static inline ProgramSources makeVtxFragSources(const std::string &v, const std::string &f)
{
	ProgramSources s;

	s.vert = v;
	s.frag = f;
	return s;
}

struct RenderContext {};

class ShaderProgram
{
public:
	ShaderProgram(const RenderContext &, const ProgramSources &src)
	{
		m_prog = glCreateProgram();
		m_vs = compile(GL_VERTEX_SHADER, src.vert);
		m_fs = compile(GL_FRAGMENT_SHADER, src.frag);
		glAttachShader(m_prog, m_vs);
		glAttachShader(m_prog, m_fs);
		glLinkProgram(m_prog);
		glGetError();          /* the link verdict is not what these cases check */
	}
	~ShaderProgram()
	{
		glDeleteShader(m_vs);
		glDeleteShader(m_fs);
		glDeleteProgram(m_prog);
	}
	GLuint getProgram() const { return m_prog; }

private:
	static GLuint compile(GLenum stage, const std::string &src)
	{
		GLuint sh = glCreateShader(stage);
		const char *s = src.c_str();

		glShaderSource(sh, 1, &s, NULL);
		glCompileShader(sh);
		return sh;
	}
	GLuint m_prog, m_vs, m_fs;
};
} // namespace glu

/* The test context the bodies reach through. */
struct DeqpContext {
	const glu::ContextInfo &getContextInfo() const { return m_info; }
	const glu::RenderContext &getRenderContext() const { return m_rc; }
	glu::ContextInfo m_info;
	glu::RenderContext m_rc;
};
extern DeqpContext m_context;


/* ---- the test context: a case that sets its own result -------------------
 * A few cases decide for themselves (a NOT_SUPPORTED skip, or a FAIL the body
 * raises without expectError). We record both. */

enum DeqpResult { QP_TEST_RESULT_PASS = 0, QP_TEST_RESULT_FAIL, QP_TEST_RESULT_NOT_SUPPORTED };
extern DeqpResult g_cur_result;

struct DeqpTestCtx {
	DeqpLogSink &getLog() { return m_log; }
	void setTestResult(DeqpResult r, const char *) { g_cur_result = r; }
};
extern DeqpTestCtx m_testCtx;

namespace glw
{
typedef GLint GLint;
typedef GLuint GLuint;
typedef GLenum GLenum;
} // namespace glw

/* ---- the deInt / de:: helpers ------------------------------------------- */

#define DE_LENGTH_OF_ARRAY(a) ((int)(sizeof(a) / sizeof((a)[0])))
#define DE_ASSERT(x) ((void)0)

static inline int deLog2Floor32(int v)
{
	int r = 0;

	while (v > 1) { v >>= 1; r++; }
	return r;
}

static inline void deMemset(void *p, int c, size_t n) { memset(p, c, n); }

namespace de
{
template <typename T> static inline T max(T a, T b) { return a > b ? a : b; }
template <typename T> static inline T min(T a, T b) { return a < b ? a : b; }
static inline bool beginsWith(const char *s, const char *p)
{
	return s && strncmp(s, p, strlen(p)) == 0;
}
} // namespace de

using std::vector;
using tcu::TestLog;
typedef int32_t deInt32;

#endif /* YAKOGL_TESTS_DEQP_SHIM_HPP */
