#include <assert.h>
#include <errno.h>
#include <limits.h>
#include <pixman.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <wordexp.h>

#include "buffer.h"
#include "grim.h"
#include "output-layout.h"
#include "render.h"
#include "write_ppm.h"
#if HAVE_JPEG
#include "write_jpg.h"
#endif
#include "write_png.h"

#include "ext-foreign-toplevel-list-v1-protocol.h"
#include "ext-image-capture-source-v1-protocol.h"
#include "ext-image-copy-capture-v1-protocol.h"
#include "wlr-screencopy-unstable-v1-protocol.h"
#include "xdg-output-unstable-v1-protocol.h"

static void screencopy_frame_handle_buffer(void *data,
		struct zwlr_screencopy_frame_v1 *frame, uint32_t format, uint32_t width,
		uint32_t height, uint32_t stride) {
	struct grim_capture *capture = data;

	capture->buffer =
		create_buffer(capture->state->shm, format, width, height, stride);
	if (capture->buffer == NULL) {
		fprintf(stderr, "failed to create buffer\n");
		exit(EXIT_FAILURE);
	}

	zwlr_screencopy_frame_v1_copy(frame, capture->buffer->wl_buffer);
}

static void screencopy_frame_handle_flags(void *data,
		struct zwlr_screencopy_frame_v1 *frame, uint32_t flags) {
	struct grim_capture *capture = data;
	capture->screencopy_frame_flags = flags;
}

static void screencopy_frame_handle_ready(void *data,
		struct zwlr_screencopy_frame_v1 *frame, uint32_t tv_sec_hi,
		uint32_t tv_sec_lo, uint32_t tv_nsec) {
	struct grim_capture *capture = data;
	++capture->state->n_done;
}

static void screencopy_frame_handle_failed(void *data,
		struct zwlr_screencopy_frame_v1 *frame) {
	struct grim_capture *capture = data;
	fprintf(stderr, "failed to copy output %s\n", capture->output->name);
	exit(EXIT_FAILURE);
}

static const struct zwlr_screencopy_frame_v1_listener screencopy_frame_listener = {
	.buffer = screencopy_frame_handle_buffer,
	.flags = screencopy_frame_handle_flags,
	.ready = screencopy_frame_handle_ready,
	.failed = screencopy_frame_handle_failed,
};


static void ext_image_copy_capture_frame_handle_transform(void *data,
		struct ext_image_copy_capture_frame_v1 *frame, uint32_t transform) {
	struct grim_capture *capture = data;
	capture->transform = transform;
}

static void ext_image_copy_capture_frame_handle_damage(void *data,
		struct ext_image_copy_capture_frame_v1 *frame, int32_t x, int32_t y,
		int32_t wdth, int32_t height) {
	// No-op
}

static void ext_image_copy_capture_frame_handle_presentation_time(void *data,
		struct ext_image_copy_capture_frame_v1 *frame, uint32_t tv_sec_hi,
		uint32_t tv_sec_lo, uint32_t tv_nsec) {
	// No-op
}

static void ext_image_copy_capture_frame_handle_ready(void *data,
		struct ext_image_copy_capture_frame_v1 *frame) {
	struct grim_capture *capture = data;
	++capture->state->n_done;
}

static void ext_image_copy_capture_frame_handle_failed(void *data,
		struct ext_image_copy_capture_frame_v1 *frame, uint32_t reason) {
	// TODO: retry depending on reason
	struct grim_capture *capture = data;
	// A foreign-toplevel capture has no output. Capture failures are normal
	// during resize/close; report them without dereferencing a NULL output.
	const char *name = capture->output && capture->output->name ? capture->output->name : "window";
	fprintf(stderr, "failed to copy %s (reason %u)\n", name, reason);
	exit(EXIT_FAILURE);
}

static const struct ext_image_copy_capture_frame_v1_listener ext_image_copy_capture_frame_listener = {
	.transform = ext_image_copy_capture_frame_handle_transform,
	.damage = ext_image_copy_capture_frame_handle_damage,
	.presentation_time = ext_image_copy_capture_frame_handle_presentation_time,
	.ready = ext_image_copy_capture_frame_handle_ready,
	.failed = ext_image_copy_capture_frame_handle_failed,
};

static void ext_image_copy_capture_session_handle_buffer_size(void *data,
		struct ext_image_copy_capture_session_v1 *session, uint32_t width, uint32_t height) {
	struct grim_capture *capture = data;
	capture->buffer_width = width;
	capture->buffer_height = height;

	if (capture->output == NULL) {
		// TODO: improve this
		capture->logical_geometry.width = width;
		capture->logical_geometry.height = height;
	}
}

static void ext_image_copy_capture_session_handle_shm_format(void *data,
		struct ext_image_copy_capture_session_v1 *session, uint32_t format) {
	struct grim_capture *capture = data;

	if (capture->has_shm_format || !is_format_supported(format)) {
		return;
	}

	capture->shm_format = format;
	capture->has_shm_format = true;
}

static void ext_image_copy_capture_session_handle_dmabuf_device(void *data,
		struct ext_image_copy_capture_session_v1 *session, struct wl_array *dev_id_array) {
	// No-op
}

static void ext_image_copy_capture_session_handle_dmabuf_format(void *data,
		struct ext_image_copy_capture_session_v1 *session, uint32_t format,
		struct wl_array *modifiers_array) {
	// No-op
}

static void ext_image_copy_capture_session_handle_done(void *data,
		struct ext_image_copy_capture_session_v1 *session) {
	struct grim_capture *capture = data;

	if (capture->ext_image_copy_capture_frame != NULL) {
		return;
	}

	if (!capture->has_shm_format) {
		fprintf(stderr, "no supported format found\n");
		exit(EXIT_FAILURE);
	}

	int32_t stride = get_format_min_stride(capture->shm_format, capture->buffer_width);
	capture->buffer =
		create_buffer(capture->state->shm, capture->shm_format, capture->buffer_width, capture->buffer_height, stride);
	if (capture->buffer == NULL) {
		fprintf(stderr, "failed to create buffer\n");
		exit(EXIT_FAILURE);
	}

	capture->ext_image_copy_capture_frame = ext_image_copy_capture_session_v1_create_frame(session);
	ext_image_copy_capture_frame_v1_add_listener(capture->ext_image_copy_capture_frame,
		&ext_image_copy_capture_frame_listener, capture);

	ext_image_copy_capture_frame_v1_attach_buffer(capture->ext_image_copy_capture_frame, capture->buffer->wl_buffer);
	ext_image_copy_capture_frame_v1_damage_buffer(capture->ext_image_copy_capture_frame,
		0, 0, INT32_MAX, INT32_MAX);
	ext_image_copy_capture_frame_v1_capture(capture->ext_image_copy_capture_frame);
}

static void ext_image_copy_capture_session_handle_stopped(void *data,
		struct ext_image_copy_capture_session_v1 *session) {
	// No-op
}

static const struct ext_image_copy_capture_session_v1_listener ext_image_copy_capture_session_listener = {
	.buffer_size = ext_image_copy_capture_session_handle_buffer_size,
	.shm_format = ext_image_copy_capture_session_handle_shm_format,
	.dmabuf_device = ext_image_copy_capture_session_handle_dmabuf_device,
	.dmabuf_format = ext_image_copy_capture_session_handle_dmabuf_format,
	.done = ext_image_copy_capture_session_handle_done,
	.stopped = ext_image_copy_capture_session_handle_stopped,
};


static void foreign_toplevel_handle_closed(void *data,
		struct ext_foreign_toplevel_handle_v1 *toplevel_handle) {
	// No-op
}

static void foreign_toplevel_handle_done(void *data,
		struct ext_foreign_toplevel_handle_v1 *toplevel_handle) {
	// TODO: wait for the done event
}

static void foreign_toplevel_handle_title(void *data,
		struct ext_foreign_toplevel_handle_v1 *toplevel_handle, const char *title) {
	// No-op
}

static void foreign_toplevel_handle_app_id(void *data,
		struct ext_foreign_toplevel_handle_v1 *toplevel_handle, const char *app_id) {
	// No-op
}

static void foreign_toplevel_handle_identifier(void *data,
		struct ext_foreign_toplevel_handle_v1 *toplevel_handle, const char *identifier) {
	struct grim_toplevel *toplevel = data;
	toplevel->identifier = strdup(identifier);
}

static const struct ext_foreign_toplevel_handle_v1_listener foreign_toplevel_listener = {
	.closed = foreign_toplevel_handle_closed,
	.done = foreign_toplevel_handle_done,
	.title = foreign_toplevel_handle_title,
	.app_id = foreign_toplevel_handle_app_id,
	.identifier = foreign_toplevel_handle_identifier,
};

static void foreign_toplevel_list_handle_toplevel(void *data,
		struct ext_foreign_toplevel_list_v1 *list,
		struct ext_foreign_toplevel_handle_v1 *toplevel_handle) {
	struct grim_state *state = data;

	struct grim_toplevel *toplevel = calloc(1, sizeof(*toplevel));
	wl_list_insert(&state->toplevels, &toplevel->link);

	toplevel->handle = toplevel_handle;

	ext_foreign_toplevel_handle_v1_add_listener(toplevel_handle, &foreign_toplevel_listener, toplevel);
}

static void foreign_toplevel_list_handle_finished(void *data,
		struct ext_foreign_toplevel_list_v1 *list) {
	// No-op
}

static const struct ext_foreign_toplevel_list_v1_listener foreign_toplevel_list_listener = {
	.toplevel = foreign_toplevel_list_handle_toplevel,
	.finished = foreign_toplevel_list_handle_finished,
};


static void xdg_output_handle_logical_position(void *data,
		struct zxdg_output_v1 *xdg_output, int32_t x, int32_t y) {
	struct grim_output *output = data;

	output->logical_geometry.x = x;
	output->logical_geometry.y = y;
}

static void xdg_output_handle_logical_size(void *data,
		struct zxdg_output_v1 *xdg_output, int32_t width, int32_t height) {
	struct grim_output *output = data;

	output->logical_geometry.width = width;
	output->logical_geometry.height = height;
}

static void xdg_output_handle_done(void *data,
		struct zxdg_output_v1 *xdg_output) {
	struct grim_output *output = data;

	// Guess the output scale from the logical size
	int32_t width = output->mode_width;
	int32_t height = output->mode_height;
	apply_output_transform(output->transform, &width, &height);
	output->logical_scale = (double)width / output->logical_geometry.width;
}

static void xdg_output_handle_name(void *data,
		struct zxdg_output_v1 *xdg_output, const char *name) {
	struct grim_output *output = data;
	if (output->name) {
		return; // prefer wl_output.name if available
	}
	output->name = strdup(name);
}

static void xdg_output_handle_description(void *data,
		struct zxdg_output_v1 *xdg_output, const char *name) {
	// No-op
}

static const struct zxdg_output_v1_listener xdg_output_listener = {
	.logical_position = xdg_output_handle_logical_position,
	.logical_size = xdg_output_handle_logical_size,
	.done = xdg_output_handle_done,
	.name = xdg_output_handle_name,
	.description = xdg_output_handle_description,
};


static void output_handle_geometry(void *data, struct wl_output *wl_output,
		int32_t x, int32_t y, int32_t physical_width, int32_t physical_height,
		int32_t subpixel, const char *make, const char *model,
		int32_t transform) {
	struct grim_output *output = data;

	output->fallback_x = x;
	output->fallback_y = y;
	output->transform = transform;
}

static void output_handle_mode(void *data, struct wl_output *wl_output,
		uint32_t flags, int32_t width, int32_t height, int32_t refresh) {
	struct grim_output *output = data;

	if ((flags & WL_OUTPUT_MODE_CURRENT) != 0) {
		output->mode_width = width;
		output->mode_height = height;
	}
}

static void output_handle_done(void *data, struct wl_output *wl_output) {
	// No-op
}

static void output_handle_scale(void *data, struct wl_output *wl_output,
		int32_t factor) {
	struct grim_output *output = data;
	output->scale = factor;
}

static void output_handle_name(void *data, struct wl_output *wl_output,
		const char *name) {
	struct grim_output *output = data;
	output->name = strdup(name);
}

static void output_handle_description(void *data, struct wl_output *wl_output,
		const char *description) {
	// No-op
}

static const struct wl_output_listener output_listener = {
	.geometry = output_handle_geometry,
	.mode = output_handle_mode,
	.done = output_handle_done,
	.scale = output_handle_scale,
	.name = output_handle_name,
	.description = output_handle_description,
};


static void handle_global(void *data, struct wl_registry *registry,
		uint32_t name, const char *interface, uint32_t version) {
	struct grim_state *state = data;

	if (strcmp(interface, wl_shm_interface.name) == 0) {
		state->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
	} else if (strcmp(interface, zxdg_output_manager_v1_interface.name) == 0) {
		uint32_t bind_version = (version > 2) ? 2 : version;
		state->xdg_output_manager = wl_registry_bind(registry, name,
			&zxdg_output_manager_v1_interface, bind_version);
	} else if (strcmp(interface, wl_output_interface.name) == 0) {
		uint32_t bind_version = (version >= 4) ? 4 : 3;
		struct grim_output *output = calloc(1, sizeof(struct grim_output));
		output->state = state;
		output->scale = 1;
		output->wl_output =  wl_registry_bind(registry, name,
			&wl_output_interface, bind_version);
		wl_output_add_listener(output->wl_output, &output_listener, output);
		wl_list_insert(&state->outputs, &output->link);
	} else if (strcmp(interface, ext_output_image_capture_source_manager_v1_interface.name) == 0) {
		state->ext_output_image_capture_source_manager = wl_registry_bind(registry, name,
			&ext_output_image_capture_source_manager_v1_interface, 1);
	} else if (strcmp(interface, ext_foreign_toplevel_image_capture_source_manager_v1_interface.name) == 0) {
		state->ext_foreign_toplevel_image_capture_source_manager = wl_registry_bind(registry, name,
			&ext_foreign_toplevel_image_capture_source_manager_v1_interface, 1);
	} else if (strcmp(interface, ext_image_copy_capture_manager_v1_interface.name) == 0) {
		state->ext_image_copy_capture_manager = wl_registry_bind(registry, name,
			&ext_image_copy_capture_manager_v1_interface, 1);
	} else if (strcmp(interface, zwlr_screencopy_manager_v1_interface.name) == 0) {
		state->screencopy_manager = wl_registry_bind(registry, name,
			&zwlr_screencopy_manager_v1_interface, 1);
	} else if (strcmp(interface, ext_foreign_toplevel_list_v1_interface.name) == 0) {
		state->foreign_toplevel_list = wl_registry_bind(registry, name,
			&ext_foreign_toplevel_list_v1_interface, 1);
		ext_foreign_toplevel_list_v1_add_listener(state->foreign_toplevel_list,
			&foreign_toplevel_list_listener, state);
	}
}

static void handle_global_remove(void *data, struct wl_registry *registry,
		uint32_t name) {
	// who cares
}

static const struct wl_registry_listener registry_listener = {
	.global = handle_global,
	.global_remove = handle_global_remove,
};

static bool default_filename(char *filename, size_t n, int filetype) {
	time_t time_epoch = time(NULL);
	struct tm *time = localtime(&time_epoch);
	if (time == NULL) {
		perror("localtime");
		return false;
	}

	char *format_str;
	const char *ext = NULL;
	switch (filetype) {
	case GRIM_FILETYPE_PNG:
		ext = "png";
		break;
	case GRIM_FILETYPE_PPM:
		ext = "ppm";
		break;
	case GRIM_FILETYPE_JPEG:
#if HAVE_JPEG
		ext = "jpeg";
		break;
#else
		abort();
#endif
	}
	assert(ext != NULL);
	char tmpstr[32];
	sprintf(tmpstr, "%%Y%%m%%d_%%Hh%%Mm%%Ss_grim.%s", ext);
	format_str = tmpstr;
	if (strftime(filename, n, format_str, time) == 0) {
		fprintf(stderr, "failed to format datetime with strftime(3)\n");
		return false;
	}
	return true;
}

static bool path_exists(const char *path) {
	return path && access(path, R_OK) != -1;
}

char *get_xdg_pictures_dir(void) {
	const char *home_dir = getenv("HOME");
	if (home_dir == NULL) {
		return NULL;
	}

	char *config_file;
	const char user_dirs_file[] = "user-dirs.dirs";
	const char config_home_fallback[] = ".config";
	const char *config_home = getenv("XDG_CONFIG_HOME");
	if (config_home == NULL || config_home[0] == 0) {
		size_t size = strlen(home_dir) + strlen("/") +
				strlen(config_home_fallback) + strlen("/") + strlen(user_dirs_file) + 1;
		config_file = malloc(size);
		if (config_file == NULL) {
			return NULL;
		}
		snprintf(config_file, size, "%s/%s/%s", home_dir, config_home_fallback, user_dirs_file);
	} else {
		size_t size = strlen(config_home) + strlen("/") + strlen(user_dirs_file) + 1;
		config_file = malloc(size);
		if (config_file == NULL) {
			return NULL;
		}
		snprintf(config_file, size, "%s/%s", config_home, user_dirs_file);
	}

	FILE *file = fopen(config_file, "r");
	free(config_file);
	if (file == NULL) {
		return NULL;
	}

	char *line = NULL;
	size_t line_size = 0;
	ssize_t nread;
	char *pictures_dir = NULL;
	while ((nread = getline(&line, &line_size, file)) != -1) {
		if (nread > 0 && line[nread - 1] == '\n') {
			line[nread - 1] = '\0';
		}

		if (strlen(line) == 0 || line[0] == '#') {
			continue;
		}

		size_t i = 0;
		while (line[i] == ' ') {
			i++;
		}
		const char prefix[] = "XDG_PICTURES_DIR=";
		if (strncmp(&line[i], prefix, strlen(prefix)) == 0) {
			const char *line_remaining = &line[i] + strlen(prefix);
			wordexp_t p;
			if (wordexp(line_remaining, &p, WRDE_UNDEF) == 0) {
				free(pictures_dir);
				pictures_dir = strdup(p.we_wordv[0]);
				wordfree(&p);
			}
		}
	}
	free(line);
	fclose(file);
	return pictures_dir;
}

char *get_output_dir(void) {
	const char *grim_default_dir = getenv("GRIM_DEFAULT_DIR");
	if (path_exists(grim_default_dir)) {
		return strdup(grim_default_dir);
	}

	char *xdg_fallback_dir = get_xdg_pictures_dir();
	if (path_exists(xdg_fallback_dir)) {
		return xdg_fallback_dir;
	} else {
		free(xdg_fallback_dir);
	}

	return strdup(".");
}

static void create_output_capture(struct grim_state *state, struct grim_output *output, bool with_cursor) {
	struct grim_capture *capture = calloc(1, sizeof(*capture));
	capture->state = state;
	capture->output = output;
	capture->transform = output->transform;
	capture->logical_geometry = output->logical_geometry;
	wl_list_insert(&state->captures, &capture->link);

	if (state->ext_output_image_capture_source_manager != NULL) {
		uint32_t options = 0;
		if (with_cursor) {
			options |= EXT_IMAGE_COPY_CAPTURE_MANAGER_V1_OPTIONS_PAINT_CURSORS;
		}
		struct ext_image_capture_source_v1 *source = ext_output_image_capture_source_manager_v1_create_source(
			state->ext_output_image_capture_source_manager, output->wl_output);
		capture->ext_image_copy_capture_session = ext_image_copy_capture_manager_v1_create_session(
			state->ext_image_copy_capture_manager, source, options);
		ext_image_copy_capture_session_v1_add_listener(capture->ext_image_copy_capture_session,
			&ext_image_copy_capture_session_listener, capture);
		ext_image_capture_source_v1_destroy(source);
	} else {
		capture->screencopy_frame = zwlr_screencopy_manager_v1_capture_output(
			state->screencopy_manager, with_cursor, output->wl_output);
		zwlr_screencopy_frame_v1_add_listener(capture->screencopy_frame,
			&screencopy_frame_listener, capture);
	}
}

static void create_toplevel_capture(struct grim_state *state, struct grim_toplevel *toplevel, bool with_cursor) {
	struct grim_capture *capture = calloc(1, sizeof(*capture));
	capture->state = state;
	wl_list_insert(&state->captures, &capture->link);

	uint32_t options = 0;
	if (with_cursor) {
		options |= EXT_IMAGE_COPY_CAPTURE_MANAGER_V1_OPTIONS_PAINT_CURSORS;
	}
	struct ext_image_capture_source_v1 *source = ext_foreign_toplevel_image_capture_source_manager_v1_create_source(
		state->ext_foreign_toplevel_image_capture_source_manager, toplevel->handle);
	struct ext_image_copy_capture_session_v1 *session = ext_image_copy_capture_manager_v1_create_session(
		state->ext_image_copy_capture_manager, source, options);
	ext_image_copy_capture_session_v1_add_listener(session,
		&ext_image_copy_capture_session_listener, capture);
	ext_image_capture_source_v1_destroy(source);
}

static const char usage[] =
	"Usage: grim [options...] [output-file]\n"
	"\n"
	"  -h              Show help message and quit.\n"
	"  -s <factor>     Set the output image scale factor. Defaults to the\n"
	"                  greatest output scale factor.\n"
	"  -g <geometry>   Set the region to capture.\n"
	"  -t png|ppm|jpeg Set the output filetype. Defaults to png.\n"
	"  -q <quality>    Set the JPEG filetype quality 0-100. Defaults to 80.\n"
	"  -l <level>      Set the PNG filetype compression level 0-9. Defaults to 6.\n"
	"  -o <output>     Set the output name to capture.\n"
	"  -T <identifier> Set the identifier of a foreign toplevel handle to capture.\n"
	"  -c              Include cursors in the screenshot.\n";

int main(int argc, char *argv[]) {
	double scale = 1.0;
	bool use_greatest_scale = true;
	struct grim_box *geometry = NULL;
	const char *geometry_output = NULL;
	enum grim_filetype output_filetype = GRIM_FILETYPE_PNG;
	int jpeg_quality = 80;
	int png_level = 6; // current default png/zlib compression level
	bool with_cursor = false;
	const char *toplevel_identifier = NULL;
	int opt;
	while ((opt = getopt(argc, argv, "hs:g:t:q:l:o:cT:")) != -1) {
		switch (opt) {
		case 'h':
			printf("%s", usage);
			return EXIT_SUCCESS;
		case 's':
			use_greatest_scale = false;
			scale = strtod(optarg, NULL);
			break;
		case 'g':;
			char *geometry_str = NULL;
			if (strcmp(optarg, "-") == 0) {
				size_t n = 0;
				ssize_t nread = getline(&geometry_str, &n, stdin);
				if (nread < 0) {
					free(geometry_str);
					fprintf(stderr, "failed to read a line from stdin\n");
					return EXIT_FAILURE;
				}

				if (nread > 0 && geometry_str[nread - 1] == '\n') {
					geometry_str[nread - 1] = '\0';
				}
			} else {
				geometry_str = strdup(optarg);
			}

			free(geometry);
			geometry = calloc(1, sizeof(struct grim_box));
			if (!parse_box(geometry, geometry_str)) {
				fprintf(stderr, "invalid geometry\n");
				return EXIT_FAILURE;
			}

			free(geometry_str);
			break;
		case 't':
			if (strcmp(optarg, "png") == 0) {
				output_filetype = GRIM_FILETYPE_PNG;
			} else if (strcmp(optarg, "ppm") == 0) {
				output_filetype = GRIM_FILETYPE_PPM;
			} else if (strcmp(optarg, "jpeg") == 0) {
#if HAVE_JPEG
				output_filetype = GRIM_FILETYPE_JPEG;
#else
				fprintf(stderr, "jpeg support disabled\n");
				return EXIT_FAILURE;
#endif
			} else {
				fprintf(stderr, "invalid filetype\n");
				return EXIT_FAILURE;
			}
			break;
		case 'q':
			if (output_filetype != GRIM_FILETYPE_JPEG) {
				fprintf(stderr, "quality is used only for jpeg files\n");
				return EXIT_FAILURE;
			} else {
				char *endptr = NULL;
				errno = 0;
				jpeg_quality = strtol(optarg, &endptr, 10);
				if (*endptr != '\0' || errno) {
					fprintf(stderr, "quality must be a integer\n");
					return EXIT_FAILURE;
				}
				if (jpeg_quality < 0 || jpeg_quality > 100) {
					fprintf(stderr, "quality valid values are between 0-100\n");
					return EXIT_FAILURE;
				}
			}
			break;
		case 'l':
			if (output_filetype != GRIM_FILETYPE_PNG) {
				fprintf(stderr, "compression level is used only for png files\n");
				return EXIT_FAILURE;
			} else {
				char *endptr = NULL;
				errno = 0;
				png_level = strtol(optarg, &endptr, 10);
				if (*endptr != '\0' || errno) {
					fprintf(stderr, "level must be a integer\n");
					return EXIT_FAILURE;
				}
				if (png_level < 0 || png_level > 9) {
					fprintf(stderr, "compression level valid values are between 0-9\n");
					return EXIT_FAILURE;
				}
			}
			break;
		case 'o':
			geometry_output = optarg;
			break;
		case 'c':
			with_cursor = true;
			break;
		case 'T':
			toplevel_identifier = optarg;
			break;
		default:
			return EXIT_FAILURE;
		}
	}

	if (geometry_output != NULL && geometry != NULL) {
		fprintf(stderr, "-o and -g are mutually exclusive\n");
		return EXIT_FAILURE;
	}
	if (geometry_output != NULL && toplevel_identifier != NULL) {
		fprintf(stderr, "-o and -T are mutually exclusive\n");
		return EXIT_FAILURE;
	}

	const char *output_filename;
	char *output_filepath;
	char tmp[64];
	if (optind >= argc) {
		if (!default_filename(tmp, sizeof(tmp), output_filetype)) {
			fprintf(stderr, "failed to generate default filename\n");
			return EXIT_FAILURE;
		}
		output_filename = tmp;

		char *output_dir = get_output_dir();
		int len = snprintf(NULL, 0, "%s/%s", output_dir, output_filename);
		if (len < 0) {
			perror("snprintf failed");
			return EXIT_FAILURE;
		}
		output_filepath = malloc(len + 1);
		snprintf(output_filepath, len + 1, "%s/%s", output_dir, output_filename);
		free(output_dir);
	} else if (optind < argc - 1) {
		printf("%s", usage);
		return EXIT_FAILURE;
	} else {
		output_filename = argv[optind];
		output_filepath = strdup(output_filename);
	}

	struct grim_state state = {0};
	wl_list_init(&state.outputs);
	wl_list_init(&state.toplevels);
	wl_list_init(&state.captures);

	state.display = wl_display_connect(NULL);
	if (state.display == NULL) {
		fprintf(stderr, "failed to create display\n");
		return EXIT_FAILURE;
	}

	state.registry = wl_display_get_registry(state.display);
	wl_registry_add_listener(state.registry, &registry_listener, &state);
	if (wl_display_roundtrip(state.display) < 0) {
		fprintf(stderr, "wl_display_roundtrip() failed\n");
		return EXIT_FAILURE;
	}

	if (state.shm == NULL) {
		fprintf(stderr, "compositor doesn't support wl_shm\n");
		return EXIT_FAILURE;
	}
	bool can_capture;
	if (toplevel_identifier != NULL) {
		can_capture = state.ext_foreign_toplevel_image_capture_source_manager != NULL && state.ext_image_copy_capture_manager;
	} else {
		can_capture = state.screencopy_manager != NULL ||
			(state.ext_output_image_capture_source_manager != NULL && state.ext_image_copy_capture_manager != NULL);;
	}
	if (!can_capture) {
		fprintf(stderr, "compositor doesn't support the screen capture protocol\n");
		return EXIT_FAILURE;
	}
	if (toplevel_identifier == NULL && wl_list_empty(&state.outputs)) {
		fprintf(stderr, "no wl_output\n");
		return EXIT_FAILURE;
	}

	if (state.xdg_output_manager != NULL) {
		struct grim_output *output;
		wl_list_for_each(output, &state.outputs, link) {
			output->xdg_output = zxdg_output_manager_v1_get_xdg_output(
				state.xdg_output_manager, output->wl_output);
			zxdg_output_v1_add_listener(output->xdg_output,
				&xdg_output_listener, output);
		}
	} else {
		fprintf(stderr, "warning: zxdg_output_manager_v1 isn't available, "
			"guessing the output layout\n");

		struct grim_output *output;
		wl_list_for_each(output, &state.outputs, link) {
			guess_output_logical_geometry(output);
		}
	}

	if (state.xdg_output_manager != NULL || toplevel_identifier != NULL) {
		if (wl_display_roundtrip(state.display) < 0) {
			fprintf(stderr, "wl_display_roundtrip() failed\n");
			return EXIT_FAILURE;
		}
	}

	if (geometry_output != NULL) {
		struct grim_output *output;
		wl_list_for_each(output, &state.outputs, link) {
			if (output->name != NULL && strcmp(output->name, geometry_output) == 0) {
				geometry = calloc(1, sizeof(struct grim_box));
				memcpy(geometry, &output->logical_geometry,
					sizeof(struct grim_box));
			}
		}

		if (geometry == NULL) {
			fprintf(stderr, "unknown output '%s'\n", geometry_output);
			return EXIT_FAILURE;
		}
	}

	if (toplevel_identifier != NULL) {
		bool found = false;
		struct grim_toplevel *toplevel;
		wl_list_for_each(toplevel, &state.toplevels, link) {
			if (strcmp(toplevel->identifier, toplevel_identifier) == 0) {
				found = true;
				break;
			}
		}
		if (!found) {
			fprintf(stderr, "cannot find toplevel\n");
			return EXIT_FAILURE;
		}

		create_toplevel_capture(&state, toplevel, with_cursor);
	} else {
		struct grim_output *output;
		wl_list_for_each(output, &state.outputs, link) {
			if (geometry != NULL &&
					!intersect_box(geometry, &output->logical_geometry)) {
				continue;
			}
			if (use_greatest_scale && output->logical_scale > scale) {
				scale = output->logical_scale;
			}

			create_output_capture(&state, output, with_cursor);
		}

		if (wl_list_empty(&state.captures)) {
			fprintf(stderr, "supplied geometry did not intersect with any outputs\n");
			return EXIT_FAILURE;
		}
	}

	size_t n_pending = wl_list_length(&state.captures);
	bool done = false;
	while (!done && wl_display_dispatch(state.display) != -1) {
		done = (state.n_done == n_pending);
	}
	if (!done) {
		fprintf(stderr, "failed to screenshoot all sources\n");
		return EXIT_FAILURE;
	}

	if (geometry == NULL) {
		geometry = calloc(1, sizeof(struct grim_box));
		get_capture_layout_extents(&state, geometry);
	}

	pixman_image_t *image = render(&state, geometry, scale);
	if (image == NULL) {
		return EXIT_FAILURE;
	}

	FILE *file;
	if (strcmp(output_filename, "-") == 0) {
		file = stdout;
	} else {
		file = fopen(output_filepath, "w");
		if (!file) {
			fprintf(stderr, "Failed to open file '%s' for writing: %s\n",
				output_filepath, strerror(errno));
			return EXIT_FAILURE;
		}
	}

	int ret = 0;
	switch (output_filetype) {
	case GRIM_FILETYPE_PPM:
		ret = write_to_ppm_stream(image, file);
		break;
	case GRIM_FILETYPE_PNG:
		ret = write_to_png_stream(image, file, png_level);
		break;
	case GRIM_FILETYPE_JPEG:
#if HAVE_JPEG
		ret = write_to_jpeg_stream(image, file, jpeg_quality);
		break;
#else
		abort();
#endif
	}
	if (ret == -1) {
		// Error messages will be printed at the source
		return EXIT_FAILURE;
	}

	if (strcmp(output_filename, "-") != 0) {
		fclose(file);
	}

	free(output_filepath);
	pixman_image_unref(image);

	struct grim_capture *capture, *capture_tmp;
	wl_list_for_each_safe(capture, capture_tmp, &state.captures, link) {
		wl_list_remove(&capture->link);
		if (capture->ext_image_copy_capture_frame != NULL) {
			ext_image_copy_capture_frame_v1_destroy(capture->ext_image_copy_capture_frame);
		}
		if (capture->ext_image_copy_capture_session != NULL) {
			ext_image_copy_capture_session_v1_destroy(capture->ext_image_copy_capture_session);
		}
		if (capture->screencopy_frame != NULL) {
			zwlr_screencopy_frame_v1_destroy(capture->screencopy_frame);
		}
		destroy_buffer(capture->buffer);
		free(capture);
	}
	struct grim_output *output, *output_tmp;
	wl_list_for_each_safe(output, output_tmp, &state.outputs, link) {
		wl_list_remove(&output->link);
		free(output->name);
		if (output->xdg_output != NULL) {
			zxdg_output_v1_destroy(output->xdg_output);
		}
		wl_output_release(output->wl_output);
		free(output);
	}
	struct grim_toplevel *toplevel, *toplevel_tmp;
	wl_list_for_each_safe(toplevel, toplevel_tmp, &state.toplevels, link) {
		wl_list_remove(&toplevel->link);
		free(toplevel->identifier);
		ext_foreign_toplevel_handle_v1_destroy(toplevel->handle);
		free(toplevel);
	}
	if (state.foreign_toplevel_list != NULL) {
		ext_foreign_toplevel_list_v1_destroy(state.foreign_toplevel_list);
	}
	if (state.ext_output_image_capture_source_manager != NULL) {
		ext_output_image_capture_source_manager_v1_destroy(state.ext_output_image_capture_source_manager);
	}
	if (state.ext_foreign_toplevel_image_capture_source_manager != NULL) {
		ext_foreign_toplevel_image_capture_source_manager_v1_destroy(state.ext_foreign_toplevel_image_capture_source_manager);
	}
	if (state.ext_image_copy_capture_manager != NULL) {
		ext_image_copy_capture_manager_v1_destroy(state.ext_image_copy_capture_manager);
	}
	if (state.screencopy_manager != NULL) {
		zwlr_screencopy_manager_v1_destroy(state.screencopy_manager);
	}
	if (state.xdg_output_manager != NULL) {
		zxdg_output_manager_v1_destroy(state.xdg_output_manager);
	}
	wl_shm_destroy(state.shm);
	wl_registry_destroy(state.registry);
	wl_display_disconnect(state.display);
	free(geometry);
	return EXIT_SUCCESS;
}
