mod layer;
mod model;
mod timekeeper;
use gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use model::{text, MINIMIZED};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    io::Read,
    os::{fd::AsRawFd, unix::net::UnixStream},
    process::{Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};

type DockRef = Rc<Dock>;
struct ClockControls {
    display: gtk::Label,
    status: gtk::Label,
    toggle: gtk::Button,
    reset: gtk::Button,
    duration_box: gtk::Box,
    minutes: gtk::SpinButton,
    seconds: gtk::SpinButton,
    mode: gtk::ComboBoxText,
}
struct App {
    info: gio::DesktopAppInfo,
    keys: HashSet<String>,
}
struct State {
    favorites: Vec<String>,
    apps: HashMap<String, App>,
    buttons: HashMap<String, gtk::Button>,
    clients: Vec<Value>,
    monitors: Vec<Value>,
    minimized: HashMap<String, i64>,
    hidden: bool,
    revealed: bool,
    dock_hovered: bool,
    preview_hovered: bool,
    preview_app: Option<String>,
}
struct Dock {
    state: RefCell<State>,
    reserve: gtk::Window,
    window: gtk::Window,
    trigger: gtk::Window,
    preview: gtk::Window,
    panel: gtk::Box,
    preview_panel: gtk::Box,
    hide_timer: RefCell<Option<glib::SourceId>>,
    close_timer: RefCell<Option<glib::SourceId>>,
    refresh_timer: RefCell<Option<glib::SourceId>>,
    refresh_pending: Cell<bool>,
    refresh_again: Cell<bool>,
    // Used only by --verify-ui to hold a preview independently of the real pointer.
    preview_test_hold: Cell<bool>,
    navigation_pending: Cell<bool>,
    snapshot_epoch: Cell<u64>,
    preview_cache: RefCell<HashMap<String, (Pixbuf, Instant)>>,
    capture_pending: RefCell<HashSet<String>>,
    capture_failed: RefCell<HashMap<String, Instant>>,
    preview_targets: RefCell<HashMap<String, (gtk::Image, gtk::Label)>>,
    clock: RefCell<timekeeper::Timekeeper>,
    clock_tick: RefCell<Option<glib::SourceId>>,
    clock_button: RefCell<Option<gtk::Button>>,
    clock_label: RefCell<Option<gtk::Label>>,
    clock_controls: RefCell<Option<ClockControls>>,
    clock_popover: RefCell<Option<gtk::Popover>>,
    controls_open: Cell<bool>,
    theme: RefCell<String>,
    theme_popover: RefCell<Option<gtk::Popover>>,
}
const THEMES: [(&str, &str, &str); 4] = [
    ("glass", "Liquid Glass", "Clear · luminous"),
    ("obsidian", "Obsidian", "Black · minimal"),
    ("frost", "Frost", "Light · soft"),
    ("midnight", "Midnight", "Blue · quiet"),
];
fn theme_path() -> std::path::PathBuf {
    model::config_path().with_file_name("appearance.json")
}
fn load_theme() -> String {
    std::fs::read_to_string(theme_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["theme"].as_str().map(String::from))
        .filter(|id| THEMES.iter().any(|(key, _, _)| key == id))
        .unwrap_or_else(|| "glass".into())
}
fn cancel(timer: &RefCell<Option<glib::SourceId>>) {
    if let Some(id) = timer.borrow_mut().take() {
        id.remove();
    }
}
fn image(app: &gio::DesktopAppInfo, size: i32) -> gtk::Image {
    let im = if let Some(icon) = app.icon() {
        gtk::Image::from_gicon(&icon, gtk::IconSize::Dialog)
    } else {
        gtk::Image::from_icon_name(Some("application-x-executable"), gtk::IconSize::Dialog)
    };
    im.set_pixel_size(size);
    im
}
fn app_keys(app: &gio::DesktopAppInfo) -> HashSet<String> {
    let mut keys = HashSet::new();
    if let Some(k) = app.string("StartupWMClass") {
        keys.insert(k.to_lowercase());
    }
    if let Some(k) = app.id() {
        keys.insert(k.trim_end_matches(".desktop").to_lowercase());
    }
    if let Some(k) = app.executable().file_name() {
        keys.insert(k.to_string_lossy().to_lowercase());
    }
    let expanded: Vec<_> = keys
        .iter()
        .map(|key| key.trim_end_matches(".desktop").to_string())
        .collect();
    keys.extend(expanded);
    keys
}
fn launch(id: &str) {
    if let Ok(exe) = std::env::current_exe() {
        match Command::new("systemd-run")
            .args(["--user", "--scope", "--quiet", "--collect"])
            .arg(exe)
            .args(["--launch", id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => eprintln!("Launch failed: {e}"),
        }
    }
}
impl Dock {
    fn new() -> DockRef {
        let favorites = std::fs::read_to_string(model::config_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| {
                v["favorites"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
            });
        let favorites = favorites
            .unwrap_or_else(|| {
                [
                    "com.mitchellh.ghostty.desktop",
                    "org.kde.dolphin.desktop",
                    "firefox.desktop",
                    "nvim.desktop",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect()
            })
            .into_iter()
            .filter(|id| gio::DesktopAppInfo::new(id).is_some())
            .collect();
        let css = gtk::CssProvider::new();
        if let Err(e) = css.load_from_data(include_bytes!("../../dock.css")) {
            eprintln!("CSS: {e}");
        }
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let reserve = layer::window("floating-dock-reserve", 1);
        reserve.set_size_request(1, 1);
        for e in [layer::LEFT, layer::RIGHT, layer::BOTTOM] {
            layer::anchor(&reserve, e);
        }
        layer::zone(&reserve, 63);
        reserve.style_context().add_class("reserve-window");
        let window = layer::window("floating-dock", 3);
        layer::anchor(&window, layer::BOTTOM);
        layer::margin(&window, layer::BOTTOM, 6);
        layer::zone(&window, -1);
        let panel = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        panel.style_context().add_class("dock-panel");
        window.add(&panel);
        let trigger = layer::window("floating-dock-trigger", 3);
        trigger.set_size_request(1, 3);
        for e in [layer::LEFT, layer::RIGHT, layer::BOTTOM] {
            layer::anchor(&trigger, e);
        }
        layer::zone(&trigger, -1);
        trigger.style_context().add_class("trigger-window");
        let preview = layer::window("floating-dock-preview", 3);
        layer::anchor(&preview, layer::BOTTOM);
        layer::margin(&preview, layer::BOTTOM, 70);
        layer::zone(&preview, -1);
        let preview_panel = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        preview_panel.style_context().add_class("preview-panel");
        preview.add(&preview_panel);
        let d = Rc::new(Self {
            state: RefCell::new(State {
                favorites,
                apps: HashMap::new(),
                buttons: HashMap::new(),
                clients: vec![],
                monitors: vec![],
                minimized: HashMap::new(),
                hidden: false,
                revealed: false,
                dock_hovered: false,
                preview_hovered: false,
                preview_app: None,
            }),
            reserve,
            window,
            trigger,
            preview,
            panel,
            preview_panel,
            hide_timer: RefCell::new(None),
            close_timer: RefCell::new(None),
            refresh_timer: RefCell::new(None),
            refresh_pending: Cell::new(false),
            refresh_again: Cell::new(false),
            preview_test_hold: Cell::new(false),
            navigation_pending: Cell::new(false),
            snapshot_epoch: Cell::new(0),
            preview_cache: RefCell::new(HashMap::new()),
            capture_pending: RefCell::new(HashSet::new()),
            capture_failed: RefCell::new(HashMap::new()),
            preview_targets: RefCell::new(HashMap::new()),
            clock: RefCell::new(timekeeper::Timekeeper::default()),
            clock_tick: RefCell::new(None),
            clock_button: RefCell::new(None),
            clock_label: RefCell::new(None),
            clock_controls: RefCell::new(None),
            clock_popover: RefCell::new(None),
            controls_open: Cell::new(false),
            theme: RefCell::new(load_theme()),
            theme_popover: RefCell::new(None),
        });
        let theme = d.theme.borrow().clone();
        d.apply_theme(&theme, false);
        d.bind_crossings();
        d.rebuild();
        d.reserve.show_all();
        d.window.show_all();
        d.connect_events();
        d.queue_refresh();
        d
    }
    fn bind_crossings(self: &DockRef) {
        let w = Rc::downgrade(self);
        self.window.connect_enter_notify_event(move |_, e| {
            if let Some(d) = w.upgrade() {
                if e.detail() != gdk::NotifyType::Inferior {
                    d.state.borrow_mut().dock_hovered = true;
                }
                cancel(&d.hide_timer);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.window.connect_leave_notify_event(move |_, e| {
            if e.detail() != gdk::NotifyType::Inferior {
                if let Some(d) = w.upgrade() {
                    d.state.borrow_mut().dock_hovered = false;
                    d.schedule_hide();
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.trigger.connect_enter_notify_event(move |_, _| {
            if let Some(d) = w.upgrade() {
                cancel(&d.hide_timer);
                if d.state.borrow().hidden {
                    d.window.show_all();
                    d.state.borrow_mut().revealed = true;
                    d.warm_previews(true);
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.trigger.connect_leave_notify_event(move |_, e| {
            if e.detail() != gdk::NotifyType::Inferior {
                if let Some(d) = w.upgrade() {
                    d.schedule_hide();
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.preview.connect_enter_notify_event(move |_, e| {
            if let Some(d) = w.upgrade() {
                if e.detail() != gdk::NotifyType::Inferior {
                    d.state.borrow_mut().preview_hovered = true;
                }
                cancel(&d.close_timer);
                cancel(&d.hide_timer);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(self);
        self.preview.connect_leave_notify_event(move |_, e| {
            if e.detail() != gdk::NotifyType::Inferior {
                if let Some(d) = w.upgrade() {
                    d.state.borrow_mut().preview_hovered = false;
                    d.schedule_close();
                    d.schedule_hide();
                }
            }
            glib::Propagation::Proceed
        });
    }
    fn save(&self) {
        if let Err(e) = model::save(&self.state.borrow().favorites) {
            eprintln!("Cannot save favorites: {e}");
        }
    }
    fn matching(&self, id: &str) -> Vec<Value> {
        let s = self.state.borrow();
        let Some(app) = s.apps.get(id) else {
            return vec![];
        };
        let mut v: Vec<_> = s
            .clients
            .iter()
            .filter(|c| model::matches(&app.keys, c))
            .cloned()
            .collect();
        v.sort_by_key(|c| {
            (
                Self::minimized_in(&s, c),
                c["focusHistoryID"].as_i64().unwrap_or(999999),
            )
        });
        v
    }
    fn minimized_in(s: &State, c: &Value) -> bool {
        s.minimized.contains_key(text(c, "address")) || text(&c["workspace"], "name") == MINIMIZED
    }
    fn is_minimized(&self, c: &Value) -> bool {
        Self::minimized_in(&self.state.borrow(), c)
    }
    fn active_workspace(&self) -> Option<i64> {
        self.state
            .borrow()
            .monitors
            .iter()
            .find(|m| m["focused"].as_bool() == Some(true))
            .and_then(|m| m["activeWorkspace"]["id"].as_i64())
            .filter(|n| *n > 0)
    }
    fn rebuild(self: &DockRef) {
        self.close_preview();
        for child in self.panel.children() {
            self.panel.remove(&child);
        }
        {
            let mut s = self.state.borrow_mut();
            s.buttons.clear();
            s.apps.clear();
        }
        let favorites = self.state.borrow().favorites.clone();
        for id in favorites {
            let Some(info) = gio::DesktopAppInfo::new(&id) else {
                continue;
            };
            let b = gtk::Button::new();
            b.set_relief(gtk::ReliefStyle::None);
            b.set_tooltip_text(Some(&info.display_name()));
            b.style_context().add_class("dock-item");
            b.add(&image(&info, 34));
            self.state.borrow_mut().apps.insert(
                id.clone(),
                App {
                    keys: app_keys(&info),
                    info,
                },
            );
            let w = Rc::downgrade(self);
            let appid = id.clone();
            b.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    d.click(&appid);
                }
            });
            let w = Rc::downgrade(self);
            let appid = id.clone();
            b.connect_button_press_event(move |_, e| {
                if e.button() == 2 {
                    launch(&appid);
                    return glib::Propagation::Stop;
                }
                if e.button() == 3 {
                    if let Some(d) = w.upgrade() {
                        d.menu(&appid, e);
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            let w = Rc::downgrade(self);
            let appid = id.clone();
            b.connect_enter_notify_event(move |_, _| {
                if let Some(d) = w.upgrade() {
                    d.hover(&appid);
                }
                glib::Propagation::Proceed
            });
            let w = Rc::downgrade(self);
            b.connect_leave_notify_event(move |_, e| {
                if e.detail() != gdk::NotifyType::Inferior {
                    if let Some(d) = w.upgrade() {
                        d.schedule_close();
                    }
                }
                glib::Propagation::Proceed
            });
            let targets = [gtk::TargetEntry::new(
                "application/x-floating-dock-item",
                gtk::TargetFlags::SAME_APP,
                0,
            )];
            b.drag_source_set(
                gdk::ModifierType::BUTTON1_MASK,
                &targets,
                gdk::DragAction::MOVE,
            );
            b.drag_dest_set(gtk::DestDefaults::ALL, &targets, gdk::DragAction::MOVE);
            let appid = id.clone();
            b.connect_drag_data_get(move |_, _, selection, _, _| {
                selection.set(&selection.target(), 8, appid.as_bytes());
            });
            let w = Rc::downgrade(self);
            let appid = id.clone();
            b.connect_drag_data_received(move |_, context, _, _, selection, _, time| {
                let ok = String::from_utf8(selection.data())
                    .ok()
                    .and_then(|source| w.upgrade().map(|d| d.reorder(&source, &appid)))
                    .unwrap_or(false);
                context.drag_finish(ok, false, time);
            });
            self.panel.pack_start(&b, false, false, 0);
            self.state.borrow_mut().buttons.insert(id, b);
        }
        let sep = gtk::Separator::new(gtk::Orientation::Vertical);
        sep.style_context().add_class("dock-separator");
        self.panel.pack_start(&sep, false, false, 5);
        let b = gtk::Button::new();
        b.set_relief(gtk::ReliefStyle::None);
        b.set_tooltip_text(Some("Add an app"));
        b.style_context().add_class("add-button");
        let im = gtk::Image::from_icon_name(Some("list-add-symbolic"), gtk::IconSize::Button);
        im.set_pixel_size(20);
        b.add(&im);
        let w = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(d) = w.upgrade() {
                d.choose_app();
            }
        });
        self.panel.pack_start(&b, false, false, 0);
        let theme_button = gtk::Button::new();
        theme_button.set_relief(gtk::ReliefStyle::None);
        theme_button.set_tooltip_text(Some("Appearance"));
        theme_button.style_context().add_class("add-button");
        theme_button.style_context().add_class("theme-button");
        let icon = gtk::Image::from_icon_name(
            Some("applications-graphics-symbolic"),
            gtk::IconSize::Button,
        );
        icon.set_pixel_size(19);
        theme_button.add(&icon);
        let weak = Rc::downgrade(self);
        theme_button.connect_clicked(move |b| {
            if let Some(d) = weak.upgrade() {
                d.show_themes(b);
            }
        });
        self.panel.pack_start(&theme_button, false, false, 0);
        self.add_clock();
        self.panel.show_all();
        self.indicators();
    }
    fn reorder(self: &DockRef, source: &str, target: &str) -> bool {
        {
            let mut s = self.state.borrow_mut();
            let Some(old) = s.favorites.iter().position(|s| s == source) else {
                return false;
            };
            let Some(new) = s.favorites.iter().position(|s| s == target) else {
                return false;
            };
            if old == new {
                return true;
            }
            let item = s.favorites.remove(old);
            s.favorites.insert(new, item);
        }
        self.save();
        self.rebuild();
        true
    }
    fn click(self: &DockRef, id: &str) {
        let clients = self.matching(id);
        if clients.is_empty() {
            launch(id);
            return;
        }
        let active = model::query("activewindow").unwrap_or(Value::Null);
        if let Some(c) = clients
            .iter()
            .find(|c| text(c, "address") == text(&active, "address") && !self.is_minimized(c))
        {
            self.minimize(c);
            return;
        }
        if let Some(c) = clients.iter().find(|c| !self.is_minimized(c)) {
            model::focus(text(c, "address"));
            self.finish_navigation();
        } else {
            self.restore(&clients[0]);
        }
    }
    fn minimize(self: &DockRef, c: &Value) {
        if self.is_minimized(c) {
            return;
        }
        let ws = c["workspace"]["id"]
            .as_i64()
            .filter(|n| *n > 0)
            .or_else(|| self.active_workspace());
        if let Some(ws) = ws {
            let addr = text(c, "address");
            if model::move_window(addr, MINIMIZED) {
                self.state.borrow_mut().minimized.insert(addr.into(), ws);
                self.close_preview();
                self.queue_refresh();
            }
        }
    }
    fn restore(self: &DockRef, c: &Value) {
        let addr = text(c, "address");
        let ws = self
            .state
            .borrow()
            .minimized
            .get(addr)
            .copied()
            .or_else(|| self.active_workspace());
        if let Some(ws) = ws {
            if model::move_window(addr, &ws.to_string()) {
                self.state.borrow_mut().minimized.remove(addr);
                model::focus(addr);
                self.finish_navigation();
            }
        }
    }
    fn focus_client(self: &DockRef, c: &Value) {
        if self.is_minimized(c) {
            self.restore(c);
        } else {
            model::focus(text(c, "address"));
            self.finish_navigation();
        }
    }
    fn finish_navigation(self: &DockRef) {
        self.close_preview();
        cancel(&self.hide_timer);
        self.navigation_pending.set(true);
        self.snapshot_epoch
            .set(self.snapshot_epoch.get().wrapping_add(1));
        {
            let mut s = self.state.borrow_mut();
            s.revealed = false;
            s.dock_hovered = false;
            s.preview_hovered = false;
        }
        self.window.hide();
        layer::zone(&self.reserve, 0);
        self.trigger.show_all();
        self.queue_refresh();
    }
    fn menu(self: &DockRef, id: &str, event: &gdk::EventButton) {
        self.close_preview();
        let menu = gtk::Menu::new();
        let item = gtk::MenuItem::with_label("Open a new window");
        let appid = id.to_owned();
        item.connect_activate(move |_| launch(&appid));
        menu.append(&item);
        let clients = self.matching(id);
        let active = model::query("activewindow").unwrap_or(Value::Null);
        if let Some(c) = clients
            .iter()
            .find(|c| text(c, "address") == text(&active, "address") && !self.is_minimized(c))
        {
            let item = gtk::MenuItem::with_label("Minimize this window");
            let c = c.clone();
            let w = Rc::downgrade(self);
            item.connect_activate(move |_| {
                if let Some(d) = w.upgrade() {
                    d.minimize(&c);
                }
            });
            menu.append(&item);
        }
        if let Some(c) = clients.iter().find(|c| self.is_minimized(c)) {
            let item = gtk::MenuItem::with_label("Restore minimized window");
            let c = c.clone();
            let w = Rc::downgrade(self);
            item.connect_activate(move |_| {
                if let Some(d) = w.upgrade() {
                    d.restore(&c);
                }
            });
            menu.append(&item);
        }
        menu.append(&gtk::SeparatorMenuItem::new());
        let item = gtk::MenuItem::with_label("Remove from Dock");
        let appid = id.to_owned();
        let w = Rc::downgrade(self);
        item.connect_activate(move |_| {
            if let Some(d) = w.upgrade() {
                d.state.borrow_mut().favorites.retain(|s| s != &appid);
                d.save();
                d.rebuild();
            }
        });
        menu.append(&item);
        menu.show_all();
        menu.popup_at_pointer(Some(event));
    }
    fn choose_app(self: &DockRef) {
        self.close_preview();
        cancel(&self.hide_timer);
        let dialog = gtk::Dialog::with_buttons(
            Some("Add to Dock"),
            Some(&self.window),
            gtk::DialogFlags::MODAL,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                ("Add", gtk::ResponseType::Ok),
            ],
        );
        dialog.set_default_size(520, 620);
        let content = dialog.content_area();
        content.set_spacing(10);
        content.set_border_width(14);
        let add = dialog.widget_for_response(gtk::ResponseType::Ok).unwrap();
        add.set_sensitive(false);
        add.style_context().add_class("suggested-action");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search applications"));
        content.pack_start(&search, false, false, 0);
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_shadow_type(gtk::ShadowType::In);
        content.pack_start(&scroll, true, true, 0);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        scroll.add(&list);
        let excluded = self.state.borrow().favorites.clone();
        let mut apps: Vec<_> = gio::AppInfo::all()
            .into_iter()
            .filter(|a| a.should_show())
            .filter_map(|a| a.downcast::<gio::DesktopAppInfo>().ok())
            .filter(|a| {
                a.id()
                    .is_some_and(|id| !excluded.iter().any(|s| s == id.as_str()))
            })
            .collect();
        apps.sort_by_key(|a| a.display_name().to_lowercase());
        let ids: Rc<Vec<String>> =
            Rc::new(apps.iter().map(|a| a.id().unwrap().to_string()).collect());
        let mut haystacks = Vec::new();
        for app in apps {
            haystacks.push(
                format!(
                    "{} {} {} {}",
                    app.display_name(),
                    app.name(),
                    app.description().unwrap_or_default(),
                    app.executable().display()
                )
                .to_lowercase(),
            );
            let row = gtk::ListBoxRow::new();
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.set_border_width(8);
            line.pack_start(&image(&app, 32), false, false, 0);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let name = gtk::Label::new(Some(&app.display_name()));
            name.set_xalign(0.0);
            name.style_context().add_class("picker-name");
            labels.pack_start(&name, false, false, 0);
            let detail = gtk::Label::new(Some(
                &app.description()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| app.executable().display().to_string()),
            ));
            detail.set_xalign(0.0);
            detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
            detail.style_context().add_class("picker-detail");
            labels.pack_start(&detail, false, false, 0);
            line.pack_start(&labels, true, true, 0);
            row.add(&line);
            list.add(&row);
        }
        let entry = search.clone();
        list.set_filter_func(Some(Box::new(move |row| {
            let query = entry.text().trim().to_lowercase();
            haystacks
                .get(row.index() as usize)
                .is_some_and(|s| query.split_whitespace().all(|term| s.contains(term)))
        })));
        let l = list.clone();
        search.connect_search_changed(move |_| l.invalidate_filter());
        let selected = Rc::new(RefCell::new(None::<String>));
        let sel = selected.clone();
        let appids = ids.clone();
        list.connect_row_selected(move |_, row| {
            *sel.borrow_mut() = row.and_then(|r| appids.get(r.index() as usize).cloned());
            add.set_sensitive(sel.borrow().is_some());
        });
        let dlg = dialog.downgrade();
        list.connect_row_activated(move |_, _| {
            if let Some(dlg) = dlg.upgrade() {
                dlg.response(gtk::ResponseType::Ok);
            }
        });
        dialog.show_all();
        search.grab_focus();
        let response = dialog.run();
        let chosen = selected.borrow().clone();
        unsafe {
            dialog.destroy();
        }
        if response == gtk::ResponseType::Ok {
            if let Some(id) = chosen {
                if !self.state.borrow().favorites.contains(&id) {
                    self.state.borrow_mut().favorites.push(id);
                    self.save();
                    self.rebuild();
                }
            }
        }
        self.schedule_hide();
    }
    fn indicators(&self) {
        let s = self.state.borrow();
        for (id, b) in &s.buttons {
            let Some(app) = s.apps.get(id) else {
                continue;
            };
            let clients: Vec<_> = s
                .clients
                .iter()
                .filter(|c| model::matches(&app.keys, c))
                .collect();
            let ctx = b.style_context();
            if clients.is_empty() {
                ctx.remove_class("running");
            } else {
                ctx.add_class("running");
            }
            if !clients.is_empty() && clients.iter().all(|c| Self::minimized_in(&s, c)) {
                ctx.add_class("minimized");
            } else {
                ctx.remove_class("minimized");
            }
        }
    }
    fn apply_snapshot(self: &DockRef, clients: Value, monitors: Value) {
        let (Some(clients), Some(monitors)) = (clients.as_array(), monitors.as_array()) else {
            return;
        };
        let hide = model::should_hide(clients, monitors);
        let changed = {
            let mut s = self.state.borrow_mut();
            s.clients = clients.clone();
            s.monitors = monitors.clone();
            s.minimized.retain(|addr, _| {
                clients.iter().any(|c| {
                    text(c, "address") == addr && text(&c["workspace"], "name") == MINIMIZED
                })
            });
            let changed = s.hidden != hide;
            s.hidden = hide;
            changed
        };
        let navigation = self.navigation_pending.replace(false);
        if changed || navigation {
            self.close_preview();
            cancel(&self.hide_timer);
            if hide {
                layer::zone(&self.reserve, 0);
                self.window.hide();
                self.trigger.show_all();
                let mut s = self.state.borrow_mut();
                s.revealed = false;
                s.dock_hovered = false;
            } else {
                self.trigger.hide();
                layer::zone(&self.reserve, 63);
                self.window.show_all();
                self.state.borrow_mut().revealed = true;
            }
        }
        self.indicators();
        let alive: HashSet<_> = clients.iter().map(preview_key).collect();
        self.preview_cache
            .borrow_mut()
            .retain(|key, _| alive.contains(key));
        self.capture_failed
            .borrow_mut()
            .retain(|key, _| alive.contains(key));
        self.warm_previews(false);
    }
    #[allow(deprecated)]
    fn queue_refresh(self: &DockRef) {
        if self.refresh_pending.get() {
            self.refresh_again.set(true);
            return;
        }
        if self.refresh_timer.borrow().is_some() {
            return;
        }
        let w = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(70), move || {
            let Some(d) = w.upgrade() else {
                return;
            };
            d.refresh_timer.borrow_mut().take();
            d.refresh_pending.set(true);
            let epoch = d.snapshot_epoch.get();
            let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
            let w = Rc::downgrade(&d);
            rx.attach(None, move |(clients, monitors)| {
                if let Some(d) = w.upgrade() {
                    d.refresh_pending.set(false);
                    if d.snapshot_epoch.get() == epoch {
                        d.apply_snapshot(clients, monitors);
                    }
                    if d.refresh_again.replace(false) {
                        d.queue_refresh();
                    }
                }
                glib::ControlFlow::Break
            });
            std::thread::spawn(move || {
                let c = model::query("clients").unwrap_or(Value::Null);
                let m = model::query("monitors").unwrap_or(Value::Null);
                let _ = tx.send((c, m));
            });
        });
        *self.refresh_timer.borrow_mut() = Some(id);
    }
    fn connect_events(self: &DockRef) {
        let stream = model::socket_path(".socket2.sock").and_then(|p| UnixStream::connect(p).ok());
        let Some(mut stream) = stream else {
            self.reconnect_later();
            return;
        };
        if stream.set_nonblocking(true).is_err() {
            self.reconnect_later();
            return;
        }
        let w = Rc::downgrade(self);
        let mut pending = String::new();
        glib::unix_fd_add_local(
            stream.as_raw_fd(),
            glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
            move |_, condition| {
                let Some(d) = w.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if condition.intersects(glib::IOCondition::HUP | glib::IOCondition::ERR) {
                    d.reconnect_later();
                    return glib::ControlFlow::Break;
                }
                let mut relevant = false;
                let mut buf = [0u8; 16384];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => {
                            d.reconnect_later();
                            return glib::ControlFlow::Break;
                        }
                        Ok(n) => {
                            pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                            while let Some(end) = pending.find('\n') {
                                relevant |= model::relevant_event(&pending[..end]);
                                pending.drain(..=end);
                            }
                            if pending.len() > 65536 {
                                pending.clear();
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            d.reconnect_later();
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                if relevant {
                    d.queue_refresh();
                }
                glib::ControlFlow::Continue
            },
        );
    }
    fn reconnect_later(self: &DockRef) {
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if let Some(d) = w.upgrade() {
                d.connect_events();
                d.queue_refresh();
            }
        });
    }
    fn schedule_hide(self: &DockRef) {
        {
            let s = self.state.borrow();
            if !s.hidden || !s.revealed || self.hide_timer.borrow().is_some() {
                return;
            }
        }
        let w = Rc::downgrade(self);
        *self.hide_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(550),
            move || {
                if let Some(d) = w.upgrade() {
                    d.hide_timer.borrow_mut().take();
                    let s = d.state.borrow();
                    let hide =
                        s.hidden && !s.dock_hovered && !s.preview_hovered && !d.controls_open.get();
                    drop(s);
                    if hide {
                        d.close_preview();
                        d.window.hide();
                        d.state.borrow_mut().revealed = false;
                    }
                }
            },
        ));
    }
    fn hover(self: &DockRef, id: &str) {
        if self.preview_test_hold.get() {
            return;
        }
        cancel(&self.close_timer);
        cancel(&self.hide_timer);
        {
            let s = self.state.borrow();
            if s.preview_app.as_deref() == Some(id) && self.preview.is_visible() {
                return;
            }
        }
        if self.matching(id).is_empty() {
            self.close_preview();
            return;
        }
        self.show_preview(id);
    }
    fn schedule_close(self: &DockRef) {
        if self.close_timer.borrow().is_some() || !self.preview.is_visible() {
            return;
        }
        let w = Rc::downgrade(self);
        *self.close_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(420),
            move || {
                if let Some(d) = w.upgrade() {
                    d.close_timer.borrow_mut().take();
                    if !d.state.borrow().preview_hovered {
                        d.close_preview();
                    }
                }
            },
        ));
    }
    fn close_preview(&self) {
        if self.preview_test_hold.get() {
            return;
        }
        cancel(&self.close_timer);
        self.preview.hide();
        self.preview_targets.borrow_mut().clear();
        for child in self.preview_panel.children() {
            self.preview_panel.remove(&child);
        }
        let mut s = self.state.borrow_mut();
        s.preview_hovered = false;
        s.preview_app = None;
    }
    #[allow(deprecated)]
    fn show_preview(self: &DockRef, id: &str) {
        let clients = self.matching(id);
        let app = self.state.borrow().apps.get(id).map(|a| a.info.clone());
        let Some(app) = app else {
            self.close_preview();
            return;
        };
        if clients.is_empty() {
            self.close_preview();
            return;
        }
        self.close_preview();
        self.state.borrow_mut().preview_app = Some(id.into());
        for c in clients.into_iter().take(6) {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 5);
            card.style_context().add_class("preview-card");
            let button = gtk::Button::new();
            button.set_relief(gtk::ReliefStyle::None);
            button.style_context().add_class("preview-image-button");
            let key = preview_key(&c);
            let cached = self
                .preview_cache
                .borrow()
                .get(&key)
                .map(|(pb, _)| pb.clone());
            let im = gtk::Image::from_pixbuf(cached.as_ref());
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 5);
            placeholder.set_size_request(230, 130);
            placeholder.pack_start(&im, true, false, 0);
            let status = gtk::Label::new(Some(if self.is_minimized(&c) {
                "Minimized"
            } else {
                "Preview unavailable"
            }));
            status.style_context().add_class("preview-status");
            placeholder.pack_start(&status, false, false, 0);
            button.add(&placeholder);
            if cached.is_some() {
                status.set_no_show_all(true);
                status.hide();
            }
            self.preview_targets.borrow_mut().insert(key, (im, status));
            let w = Rc::downgrade(self);
            let client = c.clone();
            button.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    d.focus_client(&client);
                }
            });
            card.pack_start(&button, false, false, 0);
            let footer = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let title = if text(&c, "title").is_empty() {
                app.display_name().to_string()
            } else {
                text(&c, "title").into()
            };
            let label = gtk::Label::new(Some(&title));
            label.set_xalign(0.0);
            label.set_max_width_chars(27);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_tooltip_text(Some(&title));
            label.style_context().add_class("preview-title");
            footer.pack_start(&label, true, true, 0);
            let action = gtk::Button::new();
            action.set_relief(gtk::ReliefStyle::None);
            action.style_context().add_class("preview-action");
            let minimized = self.is_minimized(&c);
            if minimized {
                card.style_context().add_class("minimized");
                action.set_tooltip_text(Some("Restore"));
                action.set_label("↗");
            } else {
                action.set_tooltip_text(Some("Minimize"));
                action.set_label("−");
            }
            let w = Rc::downgrade(self);
            let client = c.clone();
            action.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    if minimized {
                        d.restore(&client);
                    } else {
                        d.minimize(&client);
                    }
                }
            });
            footer.pack_end(&action, false, false, 0);
            card.pack_start(&footer, false, false, 0);
            self.preview_panel.pack_start(&card, false, false, 0);
        }
        self.preview.show_all();
        self.warm_previews(true);
    }
    #[allow(deprecated)]
    fn warm_previews(self: &DockRef, refresh: bool) {
        let favorites = self.state.borrow().favorites.clone();
        let active = self.state.borrow().preview_app.clone();
        let mut clients = active
            .as_deref()
            .map(|id| self.matching(id))
            .unwrap_or_default();
        for id in favorites {
            clients.extend(self.matching(&id));
        }
        let mut seen = HashSet::new();
        for client in clients
            .into_iter()
            .filter(|c| seen.insert(preview_key(c)))
            .take(24)
        {
            if self.capture_pending.borrow().len() >= 4 {
                break;
            }
            let key = preview_key(&client);
            if self.capture_pending.borrow().contains(&key) {
                continue;
            }
            if self
                .preview_cache
                .borrow()
                .get(&key)
                .is_some_and(|(_, at)| !refresh || at.elapsed() < Duration::from_secs(2))
            {
                continue;
            }
            if self
                .capture_failed
                .borrow()
                .get(&key)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
            {
                continue;
            }
            let Some(args) =
                capture_args(&client, self.is_minimized(&client), self.active_workspace())
            else {
                continue;
            };
            self.capture_pending.borrow_mut().insert(key.clone());
            let (tx, rx) =
                glib::MainContext::channel::<(String, Option<Vec<u8>>)>(glib::Priority::DEFAULT);
            let weak = Rc::downgrade(self);
            rx.attach(None, move |(key, bytes)| {
                if let Some(d) = weak.upgrade() {
                    d.capture_pending.borrow_mut().remove(&key);
                    if let Some(pb) = bytes.as_deref().and_then(decode_preview) {
                        if d.preview_cache.borrow().len() >= 24
                            && !d.preview_cache.borrow().contains_key(&key)
                        {
                            let oldest = d
                                .preview_cache
                                .borrow()
                                .iter()
                                .min_by_key(|(_, (_, at))| *at)
                                .map(|(k, _)| k.clone());
                            if let Some(oldest) = oldest {
                                d.preview_cache.borrow_mut().remove(&oldest);
                            }
                        }
                        d.preview_cache
                            .borrow_mut()
                            .insert(key.clone(), (pb.clone(), Instant::now()));
                        if let Some((image, status)) = d.preview_targets.borrow().get(&key) {
                            image.set_from_pixbuf(Some(&pb));
                            status.hide();
                        }
                        d.capture_failed.borrow_mut().remove(&key);
                    } else {
                        d.capture_failed.borrow_mut().insert(key, Instant::now());
                    }
                    d.warm_previews(false);
                }
                glib::ControlFlow::Break
            });
            std::thread::spawn(move || {
                let bytes = capture(&args);
                let _ = tx.send((key, bytes));
            });
        }
    }
    fn apply_theme(&self, theme: &str, persist: bool) {
        if !THEMES.iter().any(|(id, _, _)| *id == theme) {
            return;
        }
        for window in [&self.window, &self.preview] {
            let ctx = window.style_context();
            for (id, _, _) in THEMES {
                ctx.remove_class(&format!("theme-{id}"));
            }
            ctx.add_class(&format!("theme-{theme}"));
        }
        *self.theme.borrow_mut() = theme.into();
        if persist {
            let path = theme_path();
            let result = std::fs::create_dir_all(path.parent().unwrap()).and_then(|_| {
                let temp = path.with_extension("json.tmp");
                std::fs::write(&temp, serde_json::json!({"theme": theme}).to_string())?;
                std::fs::rename(temp, path)
            });
            if let Err(error) = result {
                eprintln!("Cannot save theme: {error}");
            }
        }
    }
    fn show_themes(self: &DockRef, button: &gtk::Button) {
        let previous = self.clock_popover.borrow().clone();
        if let Some(p) = previous {
            p.popdown();
        }
        if let Some(p) = self.theme_popover.borrow().as_ref() {
            p.popup();
            return;
        }
        self.close_preview();
        cancel(&self.hide_timer);
        self.controls_open.set(true);
        layer::keyboard(&self.window, 2);
        self.window.set_accept_focus(true);
        let popup = gtk::Popover::new(Some(button));
        popup.set_position(gtk::PositionType::Top);
        popup.style_context().add_class("appearance-popover");
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 5);
        panel.set_border_width(12);
        let heading = gtk::Label::new(Some("Appearance"));
        heading.set_xalign(0.0);
        heading.style_context().add_class("appearance-heading");
        panel.pack_start(&heading, false, false, 5);
        for (id, name, description) in THEMES {
            let row = gtk::Button::new();
            row.style_context().add_class("theme-option");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let swatch = gtk::Label::new(None);
            swatch.set_size_request(30, 30);
            swatch.style_context().add_class("theme-swatch");
            swatch.style_context().add_class(&format!("swatch-{id}"));
            content.pack_start(&swatch, false, false, 0);
            let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let title = gtk::Label::new(Some(name));
            title.set_xalign(0.0);
            let detail = gtk::Label::new(Some(description));
            detail.set_xalign(0.0);
            detail.style_context().add_class("theme-description");
            words.pack_start(&title, false, false, 0);
            words.pack_start(&detail, false, false, 0);
            content.pack_start(&words, true, true, 0);
            if self.theme.borrow().as_str() == id {
                content.pack_end(&gtk::Label::new(Some("✓")), false, false, 4);
            }
            row.add(&content);
            let weak = Rc::downgrade(self);
            row.connect_clicked(move |_| {
                if let Some(d) = weak.upgrade() {
                    d.apply_theme(id, true);
                    let p = d.theme_popover.borrow().clone();
                    if let Some(p) = p {
                        p.popdown();
                    }
                }
            });
            panel.pack_start(&row, false, false, 0);
        }
        popup.add(&panel);
        let weak = Rc::downgrade(self);
        popup.connect_closed(move |p| {
            if let Some(d) = weak.upgrade() {
                d.theme_popover.borrow_mut().take();
                d.controls_open.set(false);
                layer::keyboard(&d.window, 0);
                d.window.set_accept_focus(false);
                unsafe {
                    p.destroy();
                }
                d.schedule_hide();
            }
        });
        *self.theme_popover.borrow_mut() = Some(popup.clone());
        popup.show_all();
        popup.popup();
    }
    fn add_clock(self: &DockRef) {
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);
        separator.style_context().add_class("dock-separator");
        self.panel.pack_start(&separator, false, false, 5);
        let button = gtk::Button::new();
        button.set_relief(gtk::ReliefStyle::None);
        button.style_context().add_class("dock-clock");
        button.set_tooltip_text(Some(
            "Timer / Stopwatch — click for controls, middle-click to start or pause",
        ));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.pack_start(
            &gtk::Image::from_icon_name(Some("alarm-symbolic"), gtk::IconSize::Button),
            false,
            false,
            0,
        );
        let label = gtk::Label::new(None);
        label.style_context().add_class("dock-clock-value");
        content.pack_start(&label, false, false, 0);
        button.add(&content);
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |b| {
            if let Some(d) = weak.upgrade() {
                d.show_clock_controls(b);
            }
        });
        let weak = Rc::downgrade(self);
        button.connect_button_press_event(move |_, event| {
            if event.button() == 2 {
                if let Some(d) = weak.upgrade() {
                    d.clock_toggle();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.panel.pack_start(&button, false, false, 0);
        self.panel.reorder_child(&separator, 0);
        self.panel.reorder_child(&button, 0);
        *self.clock_button.borrow_mut() = Some(button);
        *self.clock_label.borrow_mut() = Some(label);
        self.update_clock();
    }
    fn show_clock_controls(self: &DockRef, button: &gtk::Button) {
        let previous = self.theme_popover.borrow().clone();
        if let Some(p) = previous {
            p.popdown();
        }
        if let Some(popover) = self.clock_popover.borrow().as_ref() {
            popover.popup();
            return;
        }
        self.close_preview();
        cancel(&self.hide_timer);
        self.controls_open.set(true);
        layer::keyboard(&self.window, 2);
        self.window.set_accept_focus(true);
        let popover = gtk::Popover::new(Some(button));
        popover.set_position(gtk::PositionType::Top);
        popover.style_context().add_class("timekeeper-popover");
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        panel.set_border_width(14);
        panel.set_size_request(240, -1);
        panel.style_context().add_class("clock-controls");
        let heading = gtk::Label::new(Some("TIMEKEEPER"));
        heading.set_xalign(0.0);
        heading.style_context().add_class("clock-heading");
        // A compact panel needs no separate title.
        heading.set_no_show_all(true);
        let mode = gtk::ComboBoxText::new();
        mode.append(Some("timer"), "Timer");
        mode.append(Some("stopwatch"), "Stopwatch");
        mode.set_active_id(Some(
            if self.clock.borrow().mode == timekeeper::Mode::Timer {
                "timer"
            } else {
                "stopwatch"
            },
        ));
        // Keep the mode model, with visible segmented controls in place of a menu.
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tabs.style_context().add_class("clock-tabs");
        let timer_tab = gtk::RadioButton::with_label("Timer");
        let stopwatch_tab = gtk::RadioButton::with_label_from_widget(&timer_tab, "Stopwatch");
        for tab in [&timer_tab, &stopwatch_tab] {
            tab.set_mode(false);
            tab.style_context().add_class("clock-tab");
            tabs.pack_start(tab, true, true, 0);
        }
        timer_tab.set_active(mode.active_id().as_deref() == Some("timer"));
        stopwatch_tab.set_active(mode.active_id().as_deref() == Some("stopwatch"));
        let m = mode.clone();
        timer_tab.connect_toggled(move |tab| {
            if tab.is_active() {
                m.set_active_id(Some("timer"));
            }
        });
        let m = mode.clone();
        stopwatch_tab.connect_toggled(move |tab| {
            if tab.is_active() {
                m.set_active_id(Some("stopwatch"));
            }
        });
        mode.connect_changed(move |m| {
            timer_tab.set_active(m.active_id().as_deref() == Some("timer"));
            stopwatch_tab.set_active(m.active_id().as_deref() == Some("stopwatch"));
        });
        panel.pack_start(&tabs, false, false, 0);
        let display = gtk::Label::new(None);
        display.style_context().add_class("clock-display");
        panel.pack_start(&display, false, false, 0);
        let status = gtk::Label::new(None);
        status.style_context().add_class("clock-status");
        panel.pack_start(&status, false, false, 0);
        let duration_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let inputs = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let minutes = gtk::SpinButton::with_range(0.0, 999.0, 1.0);
        minutes.set_numeric(true);
        minutes.set_width_chars(3);
        let seconds = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        seconds.set_numeric(true);
        seconds.set_width_chars(2);
        let duration = self.clock.borrow().duration.as_secs();
        minutes.set_value((duration / 60) as f64);
        seconds.set_value((duration % 60) as f64);
        for (input, caption) in [(&minutes, "MIN"), (&seconds, "SEC")] {
            let field = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let label = gtk::Label::new(Some(caption));
            label.set_xalign(0.0);
            label.style_context().add_class("clock-field-label");
            field.pack_start(&label, false, false, 0);
            field.pack_start(input, false, false, 0);
            inputs.pack_start(&field, true, true, 0);
        }
        duration_box.pack_start(&inputs, false, false, 0);
        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for value in [5, 15, 25, 45] {
            let preset = gtk::Button::with_label(&format!("{value}m"));
            preset.style_context().add_class("clock-preset");
            let weak = Rc::downgrade(self);
            preset.connect_clicked(move |_| {
                if let Some(d) = weak.upgrade() {
                    let controls = d.clock_controls.borrow();
                    if let Some(c) = controls.as_ref() {
                        c.minutes.set_value(value as f64);
                        c.seconds.set_value(0.0);
                    }
                }
            });
            presets.pack_start(&preset, true, true, 0);
        }
        duration_box.pack_start(&presets, false, false, 0);
        panel.pack_start(&duration_box, false, false, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let toggle = gtk::Button::with_label("Start");
        toggle.style_context().add_class("clock-primary");
        let reset = gtk::Button::with_label("Reset");
        reset.style_context().add_class("clock-reset");
        actions.pack_start(&toggle, true, true, 0);
        actions.pack_start(&reset, true, true, 0);
        panel.pack_start(&actions, false, false, 0);
        let weak = Rc::downgrade(self);
        toggle.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                d.clock_toggle();
            }
        });
        let weak = Rc::downgrade(self);
        reset.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                d.clock.borrow_mut().reset();
                cancel(&d.clock_tick);
                d.update_clock();
            }
        });
        let weak = Rc::downgrade(self);
        mode.connect_changed(move |combo| {
            if let Some(d) = weak.upgrade() {
                let mode = if combo.active_id().as_deref() == Some("stopwatch") {
                    timekeeper::Mode::Stopwatch
                } else {
                    timekeeper::Mode::Timer
                };
                d.clock.borrow_mut().set_mode(mode, Instant::now());
                cancel(&d.clock_tick);
                d.update_clock();
            }
        });
        let weak = Rc::downgrade(self);
        let m = minutes.clone();
        let sec = seconds.clone();
        minutes.connect_value_changed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.set_clock_duration(m.value_as_int() as u64 * 60 + sec.value_as_int() as u64);
            }
        });
        let weak = Rc::downgrade(self);
        let m = minutes.clone();
        let sec = seconds.clone();
        seconds.connect_value_changed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.set_clock_duration(m.value_as_int() as u64 * 60 + sec.value_as_int() as u64);
            }
        });
        *self.clock_controls.borrow_mut() = Some(ClockControls {
            display,
            status,
            toggle,
            reset,
            duration_box,
            minutes,
            seconds,
            mode,
        });
        popover.add(&panel);
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |p| {
            if let Some(d) = weak.upgrade() {
                d.controls_open.set(false);
                layer::keyboard(&d.window, 0);
                d.window.set_accept_focus(false);
                d.clock_controls.borrow_mut().take();
                d.clock_popover.borrow_mut().take();
                unsafe {
                    p.destroy();
                }
                d.schedule_hide();
            }
        });
        *self.clock_popover.borrow_mut() = Some(popover.clone());
        popover.show_all();
        self.update_clock();
        popover.popup();
    }
    fn set_clock_duration(self: &DockRef, seconds: u64) {
        self.clock
            .borrow_mut()
            .set_duration(seconds, Instant::now());
        cancel(&self.clock_tick);
        self.update_clock();
    }
    fn clock_toggle(self: &DockRef) {
        self.clock.borrow_mut().toggle(Instant::now());
        self.update_clock();
        if self.clock.borrow().running() && self.clock_tick.borrow().is_none() {
            let weak = Rc::downgrade(self);
            *self.clock_tick.borrow_mut() = Some(glib::timeout_add_local(
                Duration::from_millis(100),
                move || {
                    let Some(d) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let finished = d.clock.borrow_mut().tick(Instant::now());
                    d.update_clock();
                    if finished {
                        d.window.error_bell();
                        if let Ok(mut child) = Command::new("notify-send")
                            .args([
                                "--app-name=Floating Dock",
                                "Timer finished",
                                "Your timer has completed.",
                            ])
                            .spawn()
                        {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                    }
                    if d.clock.borrow().running() {
                        glib::ControlFlow::Continue
                    } else {
                        d.clock_tick.borrow_mut().take();
                        glib::ControlFlow::Break
                    }
                },
            ));
        } else if !self.clock.borrow().running() {
            cancel(&self.clock_tick);
        }
        if self.clock.borrow().running() {
            let popup = self.clock_popover.borrow().clone();
            if let Some(popup) = popup {
                popup.popdown();
            }
        }
    }
    fn update_clock(&self) {
        let clock = self.clock.borrow();
        let text = clock.label(Instant::now());
        if let Some(label) = self.clock_label.borrow().as_ref() {
            if label.text().as_str() != text {
                label.set_text(&text);
            }
        }
        if let Some(button) = self.clock_button.borrow().as_ref() {
            let ctx = button.style_context();
            if clock.running() {
                ctx.add_class("running");
            } else {
                ctx.remove_class("running");
            }
            if clock.finished {
                ctx.add_class("finished");
            } else {
                ctx.remove_class("finished");
            }
        }
        if let Some(c) = self.clock_controls.borrow().as_ref() {
            if c.display.text().as_str() != text {
                c.display.set_text(&text);
            }
            let action = if clock.running() {
                "Pause"
            } else if clock.finished {
                "Restart"
            } else {
                "Start"
            };
            if c.toggle.label().as_deref() != Some(action) {
                c.toggle.set_label(action);
            }
            let status = if clock.finished {
                "Time is up"
            } else if clock.running() {
                "Running"
            } else {
                "Ready when you are"
            };
            if c.status.text().as_str() != status {
                c.status.set_text(status);
            }
            c.duration_box
                .set_no_show_all(clock.mode == timekeeper::Mode::Stopwatch);
            c.duration_box
                .set_visible(clock.mode == timekeeper::Mode::Timer);
        }
    }
}
fn preview_key(client: &Value) -> String {
    let id = if text(client, "stableId").is_empty() {
        text(client, "address")
    } else {
        text(client, "stableId")
    };
    id.to_string()
}
fn capture_args(c: &Value, minimized: bool, workspace: Option<i64>) -> Option<Vec<String>> {
    let stable = text(c, "stableId");
    if !stable.is_empty() && stable.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(vec![
            "-t".into(),
            "ppm".into(),
            "-T".into(),
            stable.into(),
            "-s".into(),
            "0.15".into(),
            "-".into(),
        ]);
    }
    if minimized || c["workspace"]["id"].as_i64() != workspace {
        return None;
    }
    let (x, y, w, h) = (
        c["at"][0].as_i64()?,
        c["at"][1].as_i64()?,
        c["size"][0].as_i64()?,
        c["size"][1].as_i64()?,
    );
    if w < 2 || h < 2 {
        return None;
    }
    Some(vec![
        "-t".into(),
        "ppm".into(),
        "-g".into(),
        format!("{x},{y} {w}x{h}"),
        "-s".into(),
        "0.15".into(),
        "-".into(),
    ])
}
fn capture_program() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("FLOATING_DOCK_CAPTURE_HELPER") {
        return path.into();
    }
    let installed = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
        .join(".local/libexec/floating-dock/grim");
    if installed.is_file() {
        return installed;
    }
    let built =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("helpers/grim-v1.5.0/build/grim");
    if built.is_file() {
        built
    } else {
        "/usr/bin/grim".into()
    }
}
fn capture(args: &[String]) -> Option<Vec<u8>> {
    let began = Instant::now();
    for attempt in 0..2 {
        if let Some(bytes) = capture_once(args) {
            return Some(bytes);
        }
        if attempt == 0 && began.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(30));
        } else {
            break;
        }
    }
    None
}
fn capture_once(args: &[String]) -> Option<Vec<u8>> {
    let mut child = Command::new(capture_program())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let pipe = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        pipe.take(16 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .ok()
            .map(|_| bytes)
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    reader.join().ok().flatten()
                } else {
                    None
                }
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(15))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}
fn parse_ppm(bytes: &[u8]) -> Option<(i32, i32, &[u8])> {
    // grim emits exactly P6, dimensions, max value and raw RGB bytes.
    // splitn preserves binary pixels even when the first pixel is whitespace.
    let mut lines = bytes.splitn(4, |b| *b == b'\n');
    if lines.next()? != b"P6" {
        return None;
    }
    let mut dimensions = std::str::from_utf8(lines.next()?)
        .ok()?
        .split_ascii_whitespace();
    let width: usize = dimensions.next()?.parse().ok()?;
    let height: usize = dimensions.next()?.parse().ok()?;
    if dimensions.next().is_some()
        || width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || lines.next()? != b"255"
    {
        return None;
    }
    let data = lines.next()?;
    let expected = width.checked_mul(height)?.checked_mul(3)?;
    if expected > 16 * 1024 * 1024 || data.len() != expected {
        return None;
    }
    Some((width as i32, height as i32, data))
}
fn decode_preview(bytes: &[u8]) -> Option<Pixbuf> {
    let (width, height, rgb) = parse_ppm(bytes)?;
    let pixels = glib::Bytes::from_owned(rgb.to_vec());
    let pb = Pixbuf::from_bytes(
        &pixels,
        gdk_pixbuf::Colorspace::Rgb,
        false,
        8,
        width,
        height,
        width * 3,
    );
    let ratio = (230.0 / width as f64).min(130.0 / height as f64);
    pb.scale_simple(
        (width as f64 * ratio).max(1.0) as i32,
        (height as f64 * ratio).max(1.0) as i32,
        gdk_pixbuf::InterpType::Bilinear,
    )
}
// Exercise real compositor operations only on a dedicated verification window.
fn verify_ui(d: DockRef) {
    let fixture = gtk::Window::new(gtk::WindowType::Toplevel);
    fixture.set_title("Floating Dock verification");
    fixture.set_default_size(400, 220);
    fixture.add(&gtk::Label::new(Some("Rust Dock verification")));
    fixture.show_all();
    glib::timeout_add_local_once(Duration::from_millis(1000), move || {
        let clients = model::query("clients").expect("clients IPC");
        let c = clients
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                text(c, "title") == "Floating Dock verification"
                    && c["pid"].as_u64() == Some(std::process::id() as u64)
            })
            .expect("dedicated fixture")
            .clone();
        let workspace = c["workspace"]["id"].as_i64().unwrap();
        d.apply_snapshot(clients, model::query("monitors").unwrap());
        d.minimize(&c);
        let clients = model::query("clients").unwrap();
        let minimized = clients
            .as_array()
            .unwrap()
            .iter()
            .find(|v| text(v, "address") == text(&c, "address"))
            .unwrap()
            .clone();
        assert_eq!(text(&minimized["workspace"], "name"), MINIMIZED);
        d.restore(&minimized);
        let clients = model::query("clients").unwrap();
        let restored = clients
            .as_array()
            .unwrap()
            .iter()
            .find(|v| text(v, "address") == text(&c, "address"))
            .unwrap();
        assert_eq!(restored["workspace"]["id"].as_i64(), Some(workspace));
        println!("PASS: minimize and restore dedicated window");
        assert!(model::dispatch(
            "hl.dsp.window.fullscreen({ mode = \"maximized\", action = \"toggle\" })"
        ));
        let clients = model::query("clients").unwrap();
        let maximized = clients
            .as_array()
            .unwrap()
            .iter()
            .find(|v| text(v, "address") == text(&c, "address"))
            .unwrap()
            .clone();
        assert!(matches!(maximized["fullscreen"].as_i64(), Some(1..=3)));
        d.apply_snapshot(clients, model::query("monitors").unwrap());
        assert!(d.state.borrow().hidden);
        d.state.borrow_mut().revealed = true;
        d.window.show_all();
        d.focus_client(&maximized);
        assert!(
            !d.window.is_visible(),
            "navigation must close a revealed Dock even when hidden state is unchanged"
        );
        d.apply_snapshot(
            model::query("clients").unwrap(),
            model::query("monitors").unwrap(),
        );
        assert!(!d.window.is_visible());
        assert!(!d.state.borrow().revealed);
        println!("PASS: preview navigation to maximized window hides the Dock");
        unsafe {
            fixture.destroy();
        }
        let favorites = d.state.borrow().favorites.clone();
        if favorites.len() >= 2 {
            assert!(d.reorder(&favorites[0], &favorites[1]));
            assert!(d.reorder(&favorites[0], &favorites[1]));
            assert_eq!(d.state.borrow().favorites, favorites);
            println!("PASS: reorder and save favorites");
        }
        // Close the modal picker through its public response signal; exercise its
        // search callback first. No application is added during verification.
        glib::timeout_add_local_once(Duration::from_millis(350), || {
            for window in gtk::Window::list_toplevels() {
                if let Ok(dialog) = window.downcast::<gtk::Dialog>() {
                    for child in dialog.content_area().children() {
                        if let Ok(search) = child.downcast::<gtk::SearchEntry>() {
                            search.set_text("ghostty");
                        }
                    }
                    dialog.response(gtk::ResponseType::Cancel);
                }
            }
        });
        d.choose_app();
        println!("PASS: application picker and search");
        let original = d.theme.borrow().clone();
        let theme_button = d
            .panel
            .children()
            .into_iter()
            .find(|w| w.style_context().has_class("theme-button"))
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap();
        for (id, _, _) in THEMES {
            d.window.show_all();
            d.show_themes(&theme_button);
            let popup = d.theme_popover.borrow().as_ref().unwrap().clone();
            let panel = popup.child().unwrap().downcast::<gtk::Box>().unwrap();
            let index = THEMES.iter().position(|(key, _, _)| key == &id).unwrap();
            let row = panel.children()[index + 1]
                .clone()
                .downcast::<gtk::Button>()
                .unwrap();
            row.emit_clicked();
            assert_eq!(load_theme(), id);
            assert!(d.window.style_context().has_class(&format!("theme-{id}")));
            assert!(!d.controls_open.get());
        }
        d.apply_theme(&original, true);
        println!("PASS: all four theme choices apply immediately and persist");
        let clients = model::query("clients").unwrap();
        d.apply_snapshot(clients, model::query("monitors").unwrap());
        let id = d
            .state
            .borrow()
            .favorites
            .iter()
            .find(|id| !d.matching(id).is_empty())
            .cloned();
        if let Some(id) = id {
            d.window.show_all();
            let clients = d.matching(&id);
            let cached = clients
                .iter()
                .take(6)
                .filter(|c| d.preview_cache.borrow().contains_key(&preview_key(c)))
                .count();
            let began = Instant::now();
            d.show_preview(&id);
            println!(
                "PASS: immediate preview layout ({:.2} ms, {cached} cached images)",
                began.elapsed().as_secs_f64() * 1000.0
            );
            d.preview_test_hold.set(true);
            // The verification does not move the physical pointer onto the Dock.
            // Keep the synthetic hover active until the screenshot is captured.
            let mut state = d.state.borrow_mut();
            state.preview_hovered = true;
            state.dock_hovered = true;
        }
        let d2 = d.clone();
        glib::timeout_add_local_once(Duration::from_millis(1800), move || {
            assert!(!d2.preview_panel.children().is_empty());
            let mut captured = 0;
            for card in d2.preview_panel.children() {
                if let Ok(card) = card.downcast::<gtk::Box>() {
                    for child in card.children() {
                        if let Ok(button) = child.downcast::<gtk::Button>() {
                            if let Some(content) = button.child() {
                                if let Ok(container) = content.downcast::<gtk::Box>() {
                                    for child in container.children() {
                                        if let Ok(image) = child.downcast::<gtk::Image>() {
                                            if image.pixbuf().is_some() {
                                                captured += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            assert!(captured > 0, "real preview image must be displayed");
            println!("PASS: running application preview cards ({captured} captured images)");
            d2.preview_test_hold.set(false);
            let id = d2.state.borrow().preview_app.clone().unwrap();
            let began = Instant::now();
            d2.show_preview(&id);
            println!(
                "PASS: cached preview display ({:.2} ms)",
                began.elapsed().as_secs_f64() * 1000.0
            );
            d2.preview_test_hold.set(true);
            d2.window.show_all();
            if let Ok(path) = std::env::var("DOCK_VERIFY_SCREENSHOT") {
                let _ = Command::new("grim").arg(path).status();
            }
            d2.preview_test_hold.set(false);
            d2.close_preview();
            verify_clock_ui(d2);
        });
    });
}
fn verify_clock_ui(d: DockRef) {
    let button = d.clock_button.borrow().as_ref().unwrap().clone();
    d.window.show_all();
    d.show_clock_controls(&button);
    let toggle = {
        let c = d.clock_controls.borrow();
        let c = c.as_ref().unwrap();
        c.mode.set_active_id(Some("stopwatch"));
        c.toggle.clone()
    };
    toggle.emit_clicked();
    glib::timeout_add_local_once(Duration::from_millis(350), move || {
        assert!(d.clock.borrow().running());
        assert_eq!(d.clock.borrow().mode, timekeeper::Mode::Stopwatch);
        assert!(
            !d.controls_open.get(),
            "Stopwatch start must dismiss the popup"
        );
        assert!(d.clock.borrow().value(Instant::now()) >= Duration::from_millis(200));
        d.window.show_all();
        d.show_clock_controls(&button);
        let (toggle, reset, mode, minutes, seconds) = {
            let c = d.clock_controls.borrow();
            let c = c.as_ref().unwrap();
            (
                c.toggle.clone(),
                c.reset.clone(),
                c.mode.clone(),
                c.minutes.clone(),
                c.seconds.clone(),
            )
        };
        toggle.emit_clicked();
        assert!(!d.clock.borrow().running());
        assert!(d.controls_open.get(), "Pause keeps the controls available");
        reset.emit_clicked();
        assert_eq!(d.clock.borrow().value(Instant::now()), Duration::ZERO);
        println!("PASS: Stopwatch start dismisses popup; reopen, pause and reset");
        mode.set_active_id(Some("timer"));
        minutes.set_value(0.0);
        seconds.set_value(1.0);
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Ok(path) = std::env::var("DOCK_VERIFY_SCREENSHOT") {
                let path = std::path::Path::new(&path).with_file_name("timekeeper.png");
                let _ = Command::new("timeout")
                    .args(["4", "grim", "-l", "0"])
                    .arg(path)
                    .status();
            }
            toggle.emit_clicked();
            assert!(d.clock.borrow().running());
            glib::timeout_add_local_once(Duration::from_millis(1200), move || {
                assert!(!d.controls_open.get(), "Timer start must dismiss the popup");
                assert!(d.clock.borrow().finished);
                assert!(!d.clock.borrow().running());
                assert_eq!(
                    d.clock_label.borrow().as_ref().unwrap().text().as_str(),
                    "00:00"
                );
                println!(
                    "PASS: Timer start dismisses popup; countdown, completion and notification"
                );
                d.clock.borrow_mut().reset();
                d.update_clock();
                assert!(!d.clock.borrow().finished);
                gtk::main_quit();
            });
        });
    });
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--launch" {
        let Some(app) = gio::DesktopAppInfo::new(&args[2]) else {
            std::process::exit(1);
        };
        if let Err(e) = app.launch(&[], None::<&gio::AppLaunchContext>) {
            eprintln!("Launch failed: {e}");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_secs(2));
        return;
    }
    if args.get(1).is_some_and(|s| s == "--diagnostics") {
        for name in ["clients", "monitors"] {
            let v = model::query(name).unwrap_or(Value::Null);
            println!("{name}: {}", v.as_array().map_or(0, |a| a.len()));
        }
        return;
    }
    gtk::init().expect("GTK initialization failed");
    if !layer::supported() {
        eprintln!("GTK Layer Shell is unavailable");
        std::process::exit(1);
    }
    glib::set_application_name("Floating Dock");
    let dock = Dock::new();
    if args.get(1).is_some_and(|s| s == "--verify-clock-ui") {
        let d = dock.clone();
        glib::timeout_add_local_once(Duration::from_secs(1), move || verify_clock_ui(d));
    }
    if args.get(1).is_some_and(|s| s == "--verify-ui") {
        verify_ui(dock.clone());
    }
    if args.get(1).is_some_and(|s| s == "--smoke-test") {
        let d = dock.clone();
        glib::timeout_add_local_once(Duration::from_secs(1), move || {
            let id = d
                .state
                .borrow()
                .favorites
                .iter()
                .find(|id| !d.matching(id).is_empty())
                .cloned();
            if let Some(id) = id {
                d.window.show_all();
                d.show_preview(&id);
            }
        });
        glib::timeout_add_local_once(Duration::from_secs(6), gtk::main_quit);
    }
    gtk::main();
    drop(dock);
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn capture_hidden_windows_uses_toplevel_protocol() {
        let c = json!({"stableId":"abcdef","workspace":{"id":-99}});
        assert!(capture_args(&c, true, Some(1))
            .unwrap()
            .contains(&"-T".to_string()));
    }
    #[test]
    fn capture_does_not_crop_other_workspaces() {
        let c = json!({"workspace":{"id":2},"at":[0,0],"size":[400,300]});
        assert!(capture_args(&c, false, Some(1)).is_none());
        assert!(capture_args(&c, true, Some(2)).is_none());
        assert!(capture_args(&c, false, Some(2))
            .unwrap()
            .contains(&"0,0 400x300".to_string()));
    }
    #[test]
    fn ppm_keeps_whitespace_valued_pixels() {
        let input = b"P6\n1 1\n255\n\n \t";
        let (w, h, rgb) = parse_ppm(input).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(rgb, b"\n \t");
    }
    #[test]
    fn ppm_rejects_truncated_and_oversized_frames() {
        assert!(parse_ppm(b"P6\n1 1\n255\n\x00\x00").is_none());
        assert!(parse_ppm(b"P6\n999999999 999999999\n255\n").is_none());
        assert!(parse_ppm(b"P6\n0 1\n255\n").is_none());
    }
    #[test]
    fn raw_rgb_decode_matches_dimensions() {
        let input = b"P6\n1 1\n255\n\xff\x00\x00";
        let image = decode_preview(input).unwrap();
        assert_eq!((image.width(), image.height()), (130, 130));
    }
}
