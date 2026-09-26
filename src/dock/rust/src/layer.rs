//! Small GTK Layer Shell boundary. All callers run on the GTK main thread.
use glib::translate::ToGlibPtr;
use gtk::prelude::*;
use std::ffi::CString;

#[link(name = "gtk-layer-shell")]
extern "C" {
    fn gtk_layer_is_supported() -> i32;
    fn gtk_layer_init_for_window(window: *mut gtk::ffi::GtkWindow);
    fn gtk_layer_set_namespace(window: *mut gtk::ffi::GtkWindow, name: *const libc::c_char);
    fn gtk_layer_set_layer(window: *mut gtk::ffi::GtkWindow, layer: i32);
    fn gtk_layer_set_keyboard_mode(window: *mut gtk::ffi::GtkWindow, mode: i32);
    fn gtk_layer_set_anchor(window: *mut gtk::ffi::GtkWindow, edge: i32, anchor: i32);
    fn gtk_layer_set_margin(window: *mut gtk::ffi::GtkWindow, edge: i32, margin: i32);
    fn gtk_layer_set_exclusive_zone(window: *mut gtk::ffi::GtkWindow, zone: i32);
}
pub const LEFT: i32 = 0;
pub const RIGHT: i32 = 1;
pub const BOTTOM: i32 = 3;
pub fn supported() -> bool {
    unsafe { gtk_layer_is_supported() != 0 }
}
pub fn window(name: &str, layer: i32) -> gtk::Window {
    let w = gtk::Window::new(gtk::WindowType::Toplevel);
    w.set_decorated(false);
    w.set_resizable(false);
    w.set_app_paintable(true);
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&w) {
        w.set_visual(screen.rgba_visual().as_ref());
    }
    w.set_accept_focus(false);
    w.set_skip_taskbar_hint(true);
    let name = CString::new(name).expect("constant namespace");
    // SAFETY: valid live GtkWindow, valid NUL-terminated name; GTK owns its copy.
    unsafe {
        gtk_layer_init_for_window(w.to_glib_none().0);
        gtk_layer_set_namespace(w.to_glib_none().0, name.as_ptr());
        gtk_layer_set_layer(w.to_glib_none().0, layer);
        gtk_layer_set_keyboard_mode(w.to_glib_none().0, 0);
    }
    w
}
pub fn anchor(w: &gtk::Window, edge: i32) {
    unsafe {
        gtk_layer_set_anchor(w.to_glib_none().0, edge, 1);
    }
}
pub fn margin(w: &gtk::Window, edge: i32, value: i32) {
    unsafe {
        gtk_layer_set_margin(w.to_glib_none().0, edge, value);
    }
}
pub fn zone(w: &gtk::Window, value: i32) {
    unsafe {
        gtk_layer_set_exclusive_zone(w.to_glib_none().0, value);
    }
}

pub fn keyboard(w: &gtk::Window, mode: i32) {
    unsafe {
        gtk_layer_set_keyboard_mode(w.to_glib_none().0, mode);
    }
}
