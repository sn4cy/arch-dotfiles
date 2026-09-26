use serde_json::Value;
use std::{
    collections::HashSet,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};
pub const MINIMIZED: &str = "special:floating-dock-minimized";
pub fn socket_path(name: &str) -> Option<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let modern = PathBuf::from(runtime)
        .join("hypr")
        .join(&signature)
        .join(name);
    if modern.exists() {
        Some(modern)
    } else {
        Some(PathBuf::from("/tmp/hypr").join(signature).join(name))
    }
}
pub fn request(command: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket_path(".socket.sock")?).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(750))).ok()?;
    s.set_write_timeout(Some(Duration::from_millis(750))).ok()?;
    s.write_all(command.as_bytes()).ok()?;
    let mut out = String::new();
    s.take(8 * 1024 * 1024).read_to_string(&mut out).ok()?;
    Some(out)
}
pub fn query(command: &str) -> Option<Value> {
    serde_json::from_str(&request(&format!("j/{command}"))?).ok()
}
pub fn valid_address(address: &str) -> bool {
    address
        .strip_prefix("0x")
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_hexdigit()))
}
pub fn dispatch(expression: &str) -> bool {
    request(&format!("dispatch {expression}")).is_some_and(|s| s.trim() == "ok")
}
pub fn focus(address: &str) {
    if valid_address(address)
        && !dispatch(&format!(
            "hl.dsp.focus({{ window = \"address:{address}\" }})"
        ))
    {
        dispatch(&format!("focuswindow address:{address}"));
    }
}
pub fn move_window(address: &str, workspace: &str) -> bool {
    if !valid_address(address) || (workspace != MINIMIZED && workspace.parse::<i64>().is_err()) {
        return false;
    }
    dispatch(&format!("hl.dsp.window.move({{ window = \"address:{address}\", workspace = \"{workspace}\", follow = false }})"))
}
pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub fn matches(keys: &HashSet<String>, client: &Value) -> bool {
    [text(client, "class"), text(client, "initialClass")]
        .iter()
        .filter(|s| !s.is_empty())
        .any(|class| {
            let class = class.to_lowercase();
            keys.iter().any(|key| {
                key == &class
                    || (!key.is_empty()
                        && (key.ends_with(&format!(".{class}"))
                            || class.ends_with(&format!(".{key}"))))
            })
        })
}
pub fn should_hide(clients: &[Value], monitors: &[Value]) -> bool {
    let Some(m) = monitors
        .iter()
        .find(|m| m["focused"].as_bool() == Some(true))
    else {
        return false;
    };
    clients.iter().any(|c| {
        c["mapped"].as_bool().unwrap_or(true)
            && !c["hidden"].as_bool().unwrap_or(false)
            && c["monitor"] == m["id"]
            && c["workspace"]["id"] == m["activeWorkspace"]["id"]
            && (!c["floating"].as_bool().unwrap_or(false)
                || matches!(c["fullscreen"].as_i64(), Some(1..=3)))
    })
}
pub fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".config"))
        .join("floating-dock/config.json")
}
pub fn save(favorites: &[String]) -> std::io::Result<()> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({"favorites":favorites}))?
        ),
    )?;
    std::fs::rename(tmp, path)
}
pub fn relevant_event(line: &str) -> bool {
    matches!(
        line.split(">>").next().unwrap_or(""),
        "openwindow"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
            | "activewindow"
            | "activewindowv2"
            | "workspace"
            | "workspacev2"
            | "focusedmon"
            | "focusedmonv2"
            | "moveworkspace"
            | "moveworkspacev2"
            | "activespecial"
            | "activespecialv2"
            | "fullscreen"
            | "changefloatingmode"
            | "windowtitle"
            | "windowtitlev2"
            | "monitoradded"
            | "monitoraddedv2"
            | "monitorremoved"
            | "monitorremovedv2"
            | "configreloaded"
            | "togglegroup"
            | "moveintogroup"
            | "moveoutofgroup"
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn addresses_reject_injection() {
        assert!(valid_address("0x123Ab"));
        for s in ["0x", "abc", "0x1\" )", "0x-1"] {
            assert!(!valid_address(s));
        }
    }
    #[test]
    fn visibility_matches_original_behavior() {
        let monitors = vec![json!({"id":0,"focused":true,"activeWorkspace":{"id":2}})];
        let mut c = json!({"monitor":0,"workspace":{"id":2},"floating":true,"fullscreen":0});
        assert!(!should_hide(&[c.clone()], &monitors));
        c["floating"] = json!(false);
        assert!(should_hide(&[c.clone()], &monitors));
        c["workspace"]["id"] = json!(3);
        assert!(!should_hide(&[c.clone()], &monitors));
        c["workspace"]["id"] = json!(2);
        c["floating"] = json!(true);
        for n in 1..=3 {
            c["fullscreen"] = json!(n);
            assert!(should_hide(&[c.clone()], &monitors));
        }
        c["hidden"] = json!(true);
        assert!(!should_hide(&[c], &monitors));
    }
    #[test]
    fn app_matching_uses_classes_not_titles() {
        let keys = HashSet::from(["org.kde.dolphin".into()]);
        assert!(matches(&keys, &json!({"class":"Dolphin"})));
        assert!(matches(&keys, &json!({"initialClass":"org.kde.dolphin"})));
        assert!(!matches(&keys, &json!({"class":"","title":"Dolphin"})));
    }
    #[test]
    fn unrelated_events_do_not_refresh() {
        assert!(relevant_event("movewindowv2>>0xabc,2"));
        assert!(!relevant_event("activelayout>>keyboard,English"));
    }
}
