// Stub for the dirtree golden harness: fonts draw nothing.
#pragma once
#include <string>
#include "vectors.h"
class FXFont {
public:
    bool initialized() { return true; }
    void dropShadow(bool) {}
    void roundCoordinates(bool) {}
    void setColour(const vec4&) {}
    void setAlpha(float) {}
    void draw(float, float, const std::string&) {}
};
struct FXFontManager {
    FXFont grab(const std::string&, int) { return FXFont(); }
};
extern FXFontManager fontmanager;
