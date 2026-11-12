// Stub for the sim golden harness: fonts draw nothing and have no size.
#pragma once
#include <string>
#include "vectors.h"
class FXFont {
public:
    bool initialized() { return true; }
    void dropShadow(bool) {}
    void roundCoordinates(bool) {}
    void alignTop(bool) {}
    void setColour(const vec4&) {}
    void setAlpha(float) {}
    float getWidth(const std::string&) { return 0.0f; }
    float getMaxHeight() { return 0.0f; }
    void draw(float, float, const std::string&) {}
};
struct FXFontManager {
    FXFont grab(const std::string&, int) { return FXFont(); }
};
extern FXFontManager fontmanager;
