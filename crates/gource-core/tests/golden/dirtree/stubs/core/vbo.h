// Stub for the dirtree golden harness.
#pragma once
#include "gl.h"
#include "vectors.h"
class quadbuf {
public:
    void add(GLuint, const vec2&, const vec2&, const vec4&) {}
};
class bloombuf {
public:
    void add(GLuint, const vec2&, const vec2&, const vec4&, const vec4&) {}
};
