// Exercise the real failure callback with the foreign-toplevel NULL output.
#define main grim_original_main
#include "main.c"
#undef main
int main(void) {
    struct grim_capture capture = {0};
    ext_image_copy_capture_frame_handle_failed(&capture, NULL, 1);
    return 0;
}
