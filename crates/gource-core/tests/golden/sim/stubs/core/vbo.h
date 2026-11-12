// Stub for the sim golden harness.
#pragma once
#include "gl.h"
#include "vectors.h"
class quadbuf_vertex {
public:
    quadbuf_vertex() {}
    quadbuf_vertex(const vec2& pos, const vec4& colour, const vec2& texcoord)
        : pos(pos), colour(colour), texcoord(texcoord) {}
    vec2 pos;
    vec4 colour;
    vec2 texcoord;
};
class quadbuf {
public:
    void add(GLuint, const vec2&, const vec2&, const vec4&) {}
    void add(GLuint, const quadbuf_vertex&, const quadbuf_vertex&, const quadbuf_vertex&, const quadbuf_vertex&) {}
};
class bloombuf {
public:
    void add(GLuint, const vec2&, const vec2&, const vec4&, const vec4&) {}
};
