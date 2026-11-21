//! The native menu bar: the macOS menu at the top of the screen, and the menu bar inside
//! the window on Windows and Linux.
//!
//! Every Lumio item sends its id to the web view as a `menu` event, and `src/lib/menubar.ts`
//! runs the same action as the right-click menu, toolbar button or shortcut. The web view
//! tells us what is possible right now (`menu_sync`), so items that need a selection are
//! greyed out and the view, panel and sort items show a tick.
//!
//! Shortcuts: only keys with Cmd/Ctrl (plus F2, F11 and Cmd+Backspace on a Mac) are given
//! to the menu. Single-letter keys such as G, P or 0–5 must keep working in text boxes, so
//! they stay in the web view; Windows shows them next to the item as a hint.

use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use tauri::menu::{
    AboutMetadataBuilder, CheckMenuItem, CheckMenuItemBuilder, IsMenuItem, Menu, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu,
    SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

const MAC: bool = cfg!(target_os = "macos");
const WINDOWS: bool = cfg!(target_os = "windows");

/// Handles to the items whose state changes, by id.
#[derive(Default)]
pub struct MenuItems {
    plain: HashMap<String, MenuItem<Wry>>,
    checks: HashMap<String, CheckMenuItem<Wry>>,
}

pub struct MenuState(pub Mutex<MenuItems>);

/// One change from the web view.
#[derive(Deserialize)]
pub struct ItemState {
    id: String,
    enabled: Option<bool>,
    checked: Option<bool>,
    text: Option<String>,
}

struct Ctx<'a> {
    app: &'a AppHandle,
    items: MenuItems,
}

impl Ctx<'_> {
    /// A plain item. `accel` is a real shortcut; `hint` only shows the key (Windows).
    fn item(&mut self, id: &str, text: &str, accel: Option<&str>, hint: Option<&str>) -> tauri::Result<MenuItem<Wry>> {
        let label = label(text, accel, hint);
        let mut b = MenuItemBuilder::with_id(id, label);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        let it = b.build(self.app)?;
        self.items.plain.insert(id.to_string(), it.clone());
        Ok(it)
    }

    fn check(&mut self, id: &str, text: &str, accel: Option<&str>, hint: Option<&str>) -> tauri::Result<CheckMenuItem<Wry>> {
        let label = label(text, accel, hint);
        let mut b = CheckMenuItemBuilder::with_id(id, label).checked(false);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        let it = b.build(self.app)?;
        self.items.checks.insert(id.to_string(), it.clone());
        Ok(it)
    }
}

/// Windows draws "text<Tab>key" as a right-aligned shortcut, so keys handled by the
/// web view can be shown there too. Other systems only show real accelerators.
fn label(text: &str, accel: Option<&str>, hint: Option<&str>) -> String {
    match (accel, hint) {
        (None, Some(h)) if WINDOWS => format!("{text}\t{h}"),
        _ => text.to_string(),
    }
}

fn sep(app: &AppHandle) -> tauri::Result<PredefinedMenuItem<Wry>> {
    PredefinedMenuItem::separator(app)
}

fn sub(app: &AppHandle, text: &str, items: &[&dyn IsMenuItem<Wry>]) -> tauri::Result<Submenu<Wry>> {
    SubmenuBuilder::new(app, text).items(items).build()
}

/// Build the menu bar and set it on the app.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let mut c = Ctx {
        app,
        items: MenuItems::default(),
    };
    let s = || sep(app);
    let version = app.package_info().version.to_string();

    // ------------------------------------------------------------ app menu (macOS only)
    let about = PredefinedMenuItem::about(
        app,
        Some("About Lumio"),
        Some(
            AboutMetadataBuilder::new()
                .name(Some("Lumio"))
                .version(Some(version.clone()))
                .copyright(Some("© Light and Alley and Lumio contributors. MIT licence."))
                .comments(Some("A lightweight, open-source photo browser and digital asset manager."))
                .website(Some(crate::updates::REPO_URL))
                .website_label(Some("Lumio on GitHub"))
                .build(),
        ),
    )?;
    let check_updates = c.item("check_updates", "Check for Updates…", None, None)?;
    let settings = c.item("settings", "Settings…", Some("CmdOrCtrl+,"), None)?;

    // ------------------------------------------------------------ File
    let new_tab = c.item("new_tab", "New Tab", Some("CmdOrCtrl+T"), None)?;
    let close_tab = c.item("close_tab", "Close Tab", Some("CmdOrCtrl+W"), None)?;
    let add_folder = c.item("add_folder", "Add Folder to Library…", Some("CmdOrCtrl+O"), None)?;
    let new_folder = c.item("new_folder", "New Folder…", Some("CmdOrCtrl+Shift+N"), None)?;
    let rescan = c.item("rescan", "Rescan Library", None, None)?;
    let new_collection = c.item("new_collection", "New Smart Collection…", None, None)?;
    let find_dupes = c.item("find_duplicates", "Find Duplicates…", None, None)?;
    let open_default = c.item("open_default", "Open in Default App", None, None)?;
    let open_with = c.item("open_with", "Open With Another App…", None, None)?;
    let reveal = c.item(
        "reveal",
        if MAC {
            "Show in Finder"
        } else if WINDOWS {
            "Show in File Explorer"
        } else {
            "Show in File Manager"
        },
        None,
        None,
    )?;
    let export_web = c.item("export_web", "Export Web-Ready Copies…", None, None)?;
    let smugmug = c.item("smugmug", "Upload to SmugMug…", None, None)?;
    let mut file: Vec<Box<dyn IsMenuItem<Wry>>> = vec![
        Box::new(new_tab),
        Box::new(close_tab),
        Box::new(s()?),
        Box::new(add_folder),
        Box::new(new_folder),
        Box::new(rescan),
        Box::new(s()?),
        Box::new(new_collection),
        Box::new(find_dupes),
        Box::new(s()?),
        Box::new(open_default),
        Box::new(open_with),
        Box::new(reveal),
        Box::new(s()?),
        Box::new(export_web),
        Box::new(smugmug),
    ];
    if !MAC {
        file.push(Box::new(s()?));
        file.push(Box::new(settings.clone()));
        file.push(Box::new(s()?));
        file.push(Box::new(c.item("quit", "Exit", Some("CmdOrCtrl+Q"), None)?));
    }
    let file_refs: Vec<&dyn IsMenuItem<Wry>> = file.iter().map(|b| b.as_ref()).collect();
    let file_menu = sub(app, "File", &file_refs)?;

    // ------------------------------------------------------------ Edit
    // Undo and Select All act on photos, or on the text box being typed in.
    let undo = c.item("undo", "Undo", Some("CmdOrCtrl+Z"), None)?;
    let history = c.item("history", "Edit History…", None, None)?;
    let pending = c.item("pending", "Moves Waiting for a Drive…", None, None)?;
    let select_all = c.item("select_all", "Select All", Some("CmdOrCtrl+A"), None)?;
    let select_none = c.item("select_none", "Deselect All", None, Some("Esc"))?;
    let copy_photos = c.item("copy_photos", "Copy Photos", None, Some("Ctrl+C"))?;
    let find = c.item("find", "Find…", Some("CmdOrCtrl+F"), None)?;
    let mut edit: Vec<Box<dyn IsMenuItem<Wry>>> = vec![Box::new(undo)];
    // Text editing comes from the system on macOS and Windows. GTK's versions would take
    // Ctrl+C from the web view (where it copies photos), so Linux leaves those keys to it.
    if !cfg!(target_os = "linux") {
        edit.push(Box::new(PredefinedMenuItem::redo(app, None)?));
        edit.push(Box::new(s()?));
        edit.push(Box::new(PredefinedMenuItem::cut(app, None)?));
        edit.push(Box::new(PredefinedMenuItem::copy(app, None)?));
        edit.push(Box::new(PredefinedMenuItem::paste(app, None)?));
    }
    edit.extend::<Vec<Box<dyn IsMenuItem<Wry>>>>(vec![
        Box::new(select_all),
        Box::new(select_none),
        Box::new(s()?),
        Box::new(copy_photos),
        Box::new(s()?),
        Box::new(find),
        Box::new(s()?),
        Box::new(history),
        Box::new(pending),
    ]);
    let edit_refs: Vec<&dyn IsMenuItem<Wry>> = edit.iter().map(|b| b.as_ref()).collect();
    let edit_menu = sub(app, "Edit", &edit_refs)?;

    // ------------------------------------------------------------ View
    let v_grid = c.check("view_grid", "Grid", None, Some("G"))?;
    let v_list = c.check("view_list", "List", None, Some("L"))?;
    let v_film = c.check("view_filmstrip", "Filmstrip", None, Some("F"))?;
    let v_map = c.check("view_map", "Map", None, Some("M"))?;
    let p_info = c.check("panel_info", "Info Panel", None, Some("I"))?;
    let p_ai = c.check("panel_ai", "Ask AI Panel", None, None)?;
    let p_hist = c.check("histogram", "Histogram in Info Panel", None, None)?;
    let recursive = c.check("recursive", "Include Subfolders", None, None)?;
    let sort_items: Vec<CheckMenuItem<Wry>> = [
        ("sort_date", "Date Taken"),
        ("sort_name", "Name"),
        ("sort_rating", "Rating"),
        ("sort_modified", "Date Modified"),
        ("sort_size", "File Size"),
    ]
    .iter()
    .map(|(id, t)| c.check(id, t, None, None))
    .collect::<tauri::Result<_>>()?;
    let sort_desc = c.check("sort_desc", "Descending", None, None)?;
    let mut sort_refs: Vec<&dyn IsMenuItem<Wry>> = sort_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let sort_sep = s()?;
    sort_refs.push(&sort_sep);
    sort_refs.push(&sort_desc);
    let sort_menu = sub(app, "Sort By", &sort_refs)?;
    let bigger = c.item("thumbs_bigger", "Larger Thumbnails", Some("CmdOrCtrl+="), None)?;
    let smaller = c.item("thumbs_smaller", "Smaller Thumbnails", Some("CmdOrCtrl+-"), None)?;
    let bg_items: Vec<CheckMenuItem<Wry>> = [
        ("bg_black", "Black"),
        ("bg_grey", "Grey"),
        ("bg_lightgrey", "Light Grey"),
        ("bg_white", "White"),
    ]
    .iter()
    .map(|(id, t)| c.check(id, t, None, None))
    .collect::<tauri::Result<_>>()?;
    let bg_refs: Vec<&dyn IsMenuItem<Wry>> = bg_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let bg_menu = sub(app, "Viewer Background", &bg_refs)?;
    let th_dark = c.check("theme_dark", "Dark", None, None)?;
    let th_light = c.check("theme_light", "Light", None, None)?;
    let theme_menu = sub(app, "Appearance", &[&th_dark, &th_light])?;
    let slideshow = c.item("slideshow", "Slideshow", None, Some("P"))?;
    let fullscreen = c.check("fullscreen", "Full Screen", Some(if MAC { "Ctrl+Cmd+F" } else { "F11" }), None)?;
    let view_menu = sub(
        app,
        "View",
        &[
            &v_grid,
            &v_list,
            &v_film,
            &v_map,
            &s()?,
            &p_info,
            &p_ai,
            &p_hist,
            &s()?,
            &sort_menu,
            &recursive,
            &s()?,
            &bigger,
            &smaller,
            &s()?,
            &bg_menu,
            &theme_menu,
            &s()?,
            &slideshow,
            &fullscreen,
        ],
    )?;

    // ------------------------------------------------------------ Photo
    let open_viewer = c.item("open_viewer", "Open in Viewer", None, Some("Space"))?;
    let view_full = c.item("view_fullscreen", "View Full Screen", None, None)?;
    let rating_items: Vec<CheckMenuItem<Wry>> = (0..=5)
        .map(|n| {
            let t = if n == 0 { "No Rating".to_string() } else { "★".repeat(n) };
            c.check(&format!("rate_{n}"), &t, None, Some(&n.to_string()))
        })
        .collect::<tauri::Result<_>>()?;
    let rating_refs: Vec<&dyn IsMenuItem<Wry>> = rating_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let rating_menu = sub(app, "Rating", &rating_refs)?;
    let label_items: Vec<CheckMenuItem<Wry>> = [
        ("label_none", "None", None),
        ("label_red", "Red", Some("6")),
        ("label_yellow", "Yellow", Some("7")),
        ("label_green", "Green", Some("8")),
        ("label_blue", "Blue", Some("9")),
        ("label_purple", "Purple", None),
    ]
    .iter()
    .map(|(id, t, h)| c.check(id, t, None, *h))
    .collect::<tauri::Result<_>>()?;
    let label_refs: Vec<&dyn IsMenuItem<Wry>> = label_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let label_menu = sub(app, "Colour Label", &label_refs)?;
    let keywords = c.item("edit_keywords", "Add Keywords…", None, None)?;
    let details = c.item("edit_details", "Edit Title and Caption…", None, None)?;
    let rot_l = c.item("rotate_left", "Rotate Left", Some("CmdOrCtrl+["), None)?;
    let rot_r = c.item("rotate_right", "Rotate Right", Some("CmdOrCtrl+]"), None)?;
    let flip_h = c.item("flip_h", "Flip Horizontal", None, None)?;
    let flip_v = c.item("flip_v", "Flip Vertical", None, None)?;
    let group = c.item("group", "Group", Some("CmdOrCtrl+G"), None)?;
    let ungroup = c.item("ungroup", "Ungroup", Some("CmdOrCtrl+Shift+G"), None)?;
    let toggle_group = c.item("toggle_group", "Open or Close Group", None, Some("S"))?;
    let rename = c.item("rename", "Rename…", Some("F2"), None)?;
    let copy_to = c.item("copy_to", "Copy To…", None, None)?;
    let move_to = c.item("move_to", "Move To…", None, None)?;
    let share_items: Vec<MenuItem<Wry>> = {
        let mut v = Vec::new();
        if !cfg!(target_os = "linux") {
            v.push(c.item(
                "share_sheet",
                if MAC {
                    "Share… (AirDrop, Messages, Mail)"
                } else {
                    "Share… (Windows share)"
                },
                None,
                None,
            )?);
        }
        v.push(c.item("share_email", "Email", None, None)?);
        v.push(c.item("share_whatsapp", "WhatsApp", None, None)?);
        v.push(c.item("share_whatsapp_web", "WhatsApp Web", None, None)?);
        v.push(c.item("share_show", "Show Copies to Share", None, None)?);
        v
    };
    let share_refs: Vec<&dyn IsMenuItem<Wry>> = share_items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let share_menu = sub(app, "Share", &share_refs)?;
    let reread = c.item("reread", "Re-read Metadata and Thumbnail", None, None)?;
    // A plain Delete key would also fire while typing in a text box, so only the Mac's
    // Cmd+Backspace is a menu shortcut; elsewhere the web view handles Delete.
    let trash = c.item(
        "trash",
        "Move to Trash",
        if MAC { Some("Cmd+Backspace") } else { None },
        Some("Del"),
    )?;
    let photo_menu = sub(
        app,
        "Photo",
        &[
            &open_viewer,
            &view_full,
            &s()?,
            &rating_menu,
            &label_menu,
            &keywords,
            &details,
            &s()?,
            &rot_l,
            &rot_r,
            &flip_h,
            &flip_v,
            &s()?,
            &group,
            &ungroup,
            &toggle_group,
            &s()?,
            &rename,
            &copy_to,
            &move_to,
            &s()?,
            &share_menu,
            &reread,
            &s()?,
            &trash,
        ],
    )?;

    // ------------------------------------------------------------ Window (macOS)
    // No Close Window item: Cmd+W closes a tab, as in a browser.
    let window_menu = if MAC {
        Some(sub(
            app,
            "Window",
            &[
                &PredefinedMenuItem::minimize(app, None)?,
                &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
                &s()?,
                &c.item("next_tab", "Show Next Tab", Some("Ctrl+Tab"), None)?,
                &c.item("prev_tab", "Show Previous Tab", Some("Ctrl+Shift+Tab"), None)?,
            ],
        )?)
    } else {
        None
    };

    // ------------------------------------------------------------ Help
    let guide = c.item("help_guide", "Lumio User Guide", None, None)?;
    let shortcuts = c.item("help_shortcuts", "Keyboard Shortcuts", Some("CmdOrCtrl+/"), None)?;
    let notes = c.item("help_releases", "Release Notes", None, None)?;
    let issue = c.item("help_issue", "Report a Problem or Suggest an Idea…", None, None)?;
    let log = c.item("help_log", "Show Lumio's Log File", None, None)?;
    let mut help: Vec<Box<dyn IsMenuItem<Wry>>> = vec![
        Box::new(guide),
        Box::new(shortcuts),
        Box::new(notes),
        Box::new(s()?),
        Box::new(issue),
        Box::new(log),
    ];
    if !MAC {
        help.push(Box::new(s()?));
        help.push(Box::new(check_updates.clone()));
        help.push(Box::new(about.clone()));
    }
    let help_refs: Vec<&dyn IsMenuItem<Wry>> = help.iter().map(|b| b.as_ref()).collect();
    let help_menu = sub(app, "Help", &help_refs)?;

    let menu = Menu::new(app)?;
    if MAC {
        let app_menu = sub(
            app,
            "Lumio",
            &[
                &about,
                &check_updates,
                &s()?,
                &settings,
                &s()?,
                &PredefinedMenuItem::services(app, None)?,
                &s()?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &s()?,
                &PredefinedMenuItem::quit(app, None)?,
            ],
        )?;
        menu.append(&app_menu)?;
    }
    menu.append_items(&[&file_menu, &edit_menu, &view_menu, &photo_menu])?;
    if let Some(w) = &window_menu {
        menu.append(w)?;
    }
    menu.append(&help_menu)?;
    app.set_menu(menu)?;

    app.manage(MenuState(Mutex::new(c.items)));
    app.on_menu_event(|app, event| on_event(app, event.id().as_ref()));
    Ok(())
}

fn on_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        "quit" => app.exit(0),
        // Check items tick themselves when clicked; the web view sends the real state
        // back after running the action (`menu_sync`), since an action can be refused.
        _ => {
            let _ = app.emit("menu", id);
        }
    }
}

/// The web view reports what is possible now: greyed-out items, ticks and changed labels.
#[tauri::command]
pub fn menu_sync(state: tauri::State<'_, MenuState>, items: Vec<ItemState>) {
    let m = state.0.lock();
    for it in items {
        if let Some(p) = m.plain.get(&it.id) {
            if let Some(en) = it.enabled {
                let _ = p.set_enabled(en);
            }
            if let Some(t) = &it.text {
                let _ = p.set_text(t);
            }
        } else if let Some(cm) = m.checks.get(&it.id) {
            if let Some(en) = it.enabled {
                let _ = cm.set_enabled(en);
            }
            if let Some(ch) = it.checked {
                let _ = cm.set_checked(ch);
            }
        }
    }
}

/// Help > Show Lumio's Log File.
#[tauri::command]
pub fn show_log(state: tauri::State<'_, std::sync::Arc<crate::state::AppState>>) -> Result<(), String> {
    let dir = state.db_path.parent().ok_or("no data folder")?;
    let log = dir.join("lumio.log");
    let target = if log.exists() { log } else { dir.to_path_buf() };
    tauri_plugin_opener::reveal_item_in_dir(target).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn hints_only_on_windows_and_only_without_a_shortcut() {
        assert_eq!(label("Rename…", Some("F2"), Some("F2")), "Rename…");
        let hinted = label("Grid", None, Some("G"));
        if cfg!(target_os = "windows") {
            assert_eq!(hinted, "Grid\tG");
        } else {
            assert_eq!(hinted, "Grid");
        }
    }
}

/// macOS adds its own "Show Tab Bar" and "Enter Full Screen" items to the View menu.
/// Lumio has its own tabs and full-screen item, so turn those off.
#[cfg(target_os = "macos")]
pub fn mac_tidy() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSString, NSUserDefaults};
    if let Some(mtm) = MainThreadMarker::new() {
        NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
    }
    NSUserDefaults::standardUserDefaults().setBool_forKey(false, &NSString::from_str("NSFullScreenMenuItemEverywhere"));
}
