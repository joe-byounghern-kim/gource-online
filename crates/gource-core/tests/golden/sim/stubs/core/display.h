// Stub for the sim golden harness: a 1280 x 720 display.
#pragma once
#include "gl.h"
#include "vectors.h"
struct SDLAppDisplay {
    int width = 1280;
    int height = 720;
    vec3 project(vec3 v) { return v; }
    void mode3D(float, float, float) {}
};
extern SDLAppDisplay display;
