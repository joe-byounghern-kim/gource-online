// Stub for the dirtree golden harness: edges are not drawn.
#pragma once
#include "core/vectors.h"
#include "core/vbo.h"
class SplineEdge {
    vec2 label_pos;
public:
    const vec2& getLabelPos() const { return label_pos; }
    void update(const vec2&, const vec4&, const vec2&, const vec4&, const vec2&) {}
    void drawToVBO(quadbuf&) const {}
    void drawShadow() const {}
    void draw() const {}
};
