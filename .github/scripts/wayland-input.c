// A mouse and keyboard for the end-to-end test on KDE Plasma (Wayland): what
// xdotool does on X11. It talks KWin's fake-input protocol (KWin lets any
// client use it when KWIN_WAYLAND_NO_PERMISSION_CHECKS=1, which only the
// test's headless KWin sets) and reads one command per line on stdin:
//
//   move X Y         the pointer to (X, Y), in the desktop's coordinates
//   button B 1|0     press or release a button (272 left, 273 right, 274 middle)
//   axis 0|1 V       scroll (0 vertical, 1 horizontal; 15 = one notch down)
//   key CODE 1|0     press or release a key (Linux key codes: 1 Esc, 28 Enter, 29 Ctrl, 125 Super)
//
// Built in CI with the protocol's code from wayland-scanner:
//   wayland-scanner client-header fake-input.xml fake-input.h
//   wayland-scanner private-code fake-input.xml fake-input.c
//   cc -o wayland-input wayland-input.c fake-input.c -lwayland-client
#include <stdio.h>
#include <string.h>
#include <wayland-client.h>
#include "fake-input.h"

static struct org_kde_kwin_fake_input *input;

static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data;
    if (strcmp(interface, org_kde_kwin_fake_input_interface.name) == 0 && version >= 4)
        input = wl_registry_bind(registry, name, &org_kde_kwin_fake_input_interface, 4);
}

static void global_remove(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data, (void)registry, (void)name;
}

static const struct wl_registry_listener listener = {global, global_remove};

int main(void) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) {
        fprintf(stderr, "wayland-input: no Wayland display\n");
        return 1;
    }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    wl_display_roundtrip(display);
    if (!input) {
        fprintf(stderr, "wayland-input: KWin offers no fake input (version 4)\n");
        return 1;
    }
    org_kde_kwin_fake_input_authenticate(input, "archtoys e2e", "the end-to-end test");
    char line[128];
    while (fgets(line, sizeof line, stdin)) {
        double x, y;
        unsigned a, b;
        if (sscanf(line, "move %lf %lf", &x, &y) == 2)
            org_kde_kwin_fake_input_pointer_motion_absolute(input, wl_fixed_from_double(x), wl_fixed_from_double(y));
        else if (sscanf(line, "button %u %u", &a, &b) == 2)
            org_kde_kwin_fake_input_button(input, a, b);
        else if (sscanf(line, "axis %u %lf", &a, &x) == 2)
            org_kde_kwin_fake_input_axis(input, a, wl_fixed_from_double(x));
        else if (sscanf(line, "key %u %u", &a, &b) == 2)
            org_kde_kwin_fake_input_keyboard_key(input, a, b);
        else {
            fprintf(stderr, "wayland-input: what is \"%s\"?\n", line);
            continue;
        }
        // each event reaches KWin before the next is read
        wl_display_roundtrip(display);
        printf("ok\n");
        fflush(stdout);
    }
    wl_display_disconnect(display);
    return 0;
}
