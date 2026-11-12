// Stub for the dirtree golden harness.
#pragma once
#include "gl.h"
#include "vectors.h"
struct SDLAppDisplay {
    int width = 1280;
    int height = 720;
    vec3 project(vec3 v) { return v; }
};
extern SDLAppDisplay display;
