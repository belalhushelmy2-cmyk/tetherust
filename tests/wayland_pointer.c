/* Development-only virtual pointer for private compositor acceptance. */
#include <wayland-client.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include "virtual-pointer.h"
static struct zwlr_virtual_pointer_manager_v1 *manager;
static struct wl_seat *seat;
static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    (void)data; (void)version;
    if (!strcmp(interface, "zwlr_virtual_pointer_manager_v1")) manager = wl_registry_bind(registry, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
    if (!strcmp(interface, "wl_seat") && !seat) seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) { (void)data; (void)registry; (void)name; }
int main(int argc, char **argv) {
    if (argc != 5) return 2;
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return 3;
    struct wl_registry *registry = wl_display_get_registry(display);
    struct wl_registry_listener listener = {global, removed};
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !manager || !seat) return 4;
    struct zwlr_virtual_pointer_v1 *pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, seat);
    zwlr_virtual_pointer_v1_motion_absolute(pointer, 1, atoi(argv[1]), atoi(argv[2]), atoi(argv[3]), atoi(argv[4]));
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display); usleep(100000);
    zwlr_virtual_pointer_v1_button(pointer, 2, 0x110, WL_POINTER_BUTTON_STATE_PRESSED);
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_flush(display); usleep(100000);
    zwlr_virtual_pointer_v1_button(pointer, 3, 0x110, WL_POINTER_BUTTON_STATE_RELEASED);
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display); usleep(150000);
    zwlr_virtual_pointer_v1_destroy(pointer);
    zwlr_virtual_pointer_manager_v1_destroy(manager);
    wl_seat_destroy(seat); wl_registry_destroy(registry);
    wl_display_roundtrip(display); wl_display_disconnect(display);
    return 0;
}
