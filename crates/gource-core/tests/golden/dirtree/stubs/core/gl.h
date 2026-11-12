// Stub for the dirtree golden harness: GL calls are no-ops.
#pragma once
// The real headers reach these through SDL and Core includes.
#include <algorithm>
#include <cmath>
#include <list>
#include <map>
#include <set>
#include <string>
#include <vector>
typedef unsigned int GLuint;
typedef int GLint;
typedef double GLdouble;
typedef unsigned int GLenum;
#define GL_LINE_STRIP 0
#define GL_LINES 1
#define GL_QUADS 2
#define GL_TEXTURE_2D 3
#define GL_COMPILE 4
inline void glBegin(GLenum) {}
inline void glEnd() {}
inline void glVertex2f(float, float) {}
inline void glVertex2fv(const float*) {}
inline void glVertex3fv(const float*) {}
inline void glTexCoord2f(float, float) {}
inline void glColor4f(float, float, float, float) {}
inline void glPushMatrix() {}
inline void glPopMatrix() {}
inline void glTranslatef(float, float, float) {}
inline void glBindTexture(GLenum, GLuint) {}
inline void glLoadName(GLuint) {}
inline void glCallList(GLuint) {}
inline GLuint glGenLists(int) { return 0; }
inline void glNewList(GLuint, GLenum) {}
inline void glEndList() {}
inline void glDeleteLists(GLuint, int) {}
inline int gluProject(double, double, double, const double*, const double*, const int*,
                      double* x, double* y, double* z) { *x = *y = *z = 0.0; return 1; }
