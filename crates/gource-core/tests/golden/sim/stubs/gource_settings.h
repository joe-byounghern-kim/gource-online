// Stub for the sim golden harness: the GourceSettings fields dirnode, file,
// pawn, user, action and zoomcamera read, with
// GourceSettings::setGourceDefaults() values.
#pragma once
#include <map>
#include <string>
#include "core/gl.h"
#include "core/vectors.h"
struct TextureResource {
    GLuint textureid = 0;
    int w = 256;
    int h = 256;
};
struct TextureManager {
    TextureResource file_png{0, 256, 256};
    TextureResource user_png{0, 384, 512};
    TextureResource* grab(const std::string& name, bool = true, GLenum = 0) {
        return name == "user.png" ? &user_png : &file_png;
    }
    TextureResource* grabFile(const std::string& name, bool = true, GLenum = 0) { return grab(name); }
};
extern TextureManager texturemanager;
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
    vec3 highlight_colour = vec3(1.0f);
    vec3 filename_colour = vec3(1.0f);
    int scaled_filename_font_size = 14;
    int scaled_user_font_size = 14;
    bool file_extension_fallback = false;
    float file_idle_time = 0.0f;
    bool file_extensions = false;
    // users and actions
    float max_user_speed = 500.0f;
    float user_scale = 1.0f;
    float max_file_lag = 5.0f;
    float user_idle_time = 3.0f;
    float user_friction = 1.0f;
    bool highlight_all_users = false;
    bool hide_users = false;
    std::string user_image_dir;
    std::map<std::string, std::string> user_image_map;
    std::string default_user_image;
    bool colour_user_images = false;
    // camera
    bool crop_vertical = false;
    bool crop_horizontal = false;
    float padding = 1.1f;
    float camera_zoom_min = 50.0f;
    float camera_zoom_default = 100.0f;
    float camera_zoom_max = 10000.0f;
    bool disable_auto_rotate = false;
    // time
    float days_per_second = 0.1f;
    float auto_skip_seconds = 3.0f;
    float time_scale = 1.0f;
    bool no_time_travel = false;
};
extern GourceSettings gGourceSettings;
