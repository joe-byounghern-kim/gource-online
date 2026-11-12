// Stub for the dirtree golden harness: only the fields dirnode/file/pawn read,
// with GourceSettings::setGourceDefaults() values.
#pragma once
#include <string>
#include "core/gl.h"
#include "core/vectors.h"
struct TextureResource {
    GLuint textureid = 0;
    int w = 256;
    int h = 256;
};
class GourceSettings {
public:
    float elasticity = 0.0f;
    bool hide_dirnames = false;
    int dir_name_depth = 0;
    bool highlight_dirs = false;
    bool hide_filenames = false;
    bool hide_files = false;
    float bloom_multiplier = 1.0f;
    float bloom_intensity = 0.75f;
    bool hide_root = false;
    TextureResource* file_graphic = nullptr;
    float filename_time = 4.0f;
    std::string font_file;
    vec3 selection_colour = vec3(1.0f, 1.0f, 0.3f);
    vec3 filename_colour = vec3(1.0f);
    int scaled_filename_font_size = 14;
    bool file_extension_fallback = false;
    float file_idle_time = 0.0f;
    bool file_extensions = false;
};
extern GourceSettings gGourceSettings;
