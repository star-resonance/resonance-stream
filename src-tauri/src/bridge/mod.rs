//! The test bridge (`--bridge-url`, see `live.rs`). It exists only in builds made
//! with the `test-env` feature: a stable release holds none of it, and not the MQTT
//! client either (`rumqttc` is an optional dependency of that feature). Without the
//! feature the three entry points below do nothing, so call sites need no `cfg`.

#[cfg(feature = "test-env")]
mod live;
#[cfg(feature = "test-env")]
pub use live::*;

#[cfg(not(feature = "test-env"))]
mod off {
    use serde_json::Value;
    use tauri::ipc::Invoke;
    use tauri::{AppHandle, Runtime};

    pub fn start(_app: &AppHandle) {}

    pub fn publish_event(_name: &str, _payload: Value) {}

    pub fn tap_commands<R: Runtime>(
        inner: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
    ) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
        inner
    }
}
#[cfg(not(feature = "test-env"))]
pub use off::*;

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn manifest() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("read Cargo.toml")
    }

    /// A stable exe must not carry the MQTT client: it is only there for the bridge.
    #[test]
    fn the_mqtt_client_is_a_dependency_of_the_test_env_feature_only() {
        let manifest = manifest();
        let line = manifest
            .lines()
            .find(|l| l.starts_with("rumqttc"))
            .expect("rumqttc is declared");
        assert!(
            line.contains("optional = true"),
            "rumqttc must be optional: {line}"
        );
        let feature = manifest
            .lines()
            .find(|l| l.starts_with("test-env"))
            .expect("test-env feature is declared");
        assert!(
            feature.contains("dep:rumqttc"),
            "test-env must enable it: {feature}"
        );
    }

    /// The commands `lib.rs` hands to `generate_handler!`, read as text.
    fn registered_commands() -> Vec<String> {
        let lib = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read lib.rs");
        let start = lib
            .find("generate_handler![")
            .expect("generate_handler! in lib.rs");
        let body = &lib[start + "generate_handler![".len()..];
        let body = &body[..body.find(']').expect("the end of generate_handler!")];
        body.lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .flat_map(|line| line.split(','))
            .map(|path| {
                path.trim()
                    .rsplit("::")
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// The bridge reaches the app through the plain functions beside the commands, never a
    /// `#[tauri::command]` itself: a command takes Tauri's `State` and may be async, which the bridge
    /// would have to fake (`app.state()`, `block_on`). It calls everything by path.
    #[test]
    fn the_bridge_calls_no_tauri_command() {
        let live = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge/live.rs"),
        )
        .expect("read live.rs");
        let commands = registered_commands();
        assert!(commands.len() > 40, "read the command list: {commands:?}");
        let called: Vec<_> = commands
            .iter()
            .filter(|name| live.contains(&format!("::{name}(")))
            .collect();
        assert!(
            called.is_empty(),
            "the bridge calls these commands: {called:?}"
        );
    }

    /// Nothing but the bridge itself may name the client, or the stub build breaks.
    #[test]
    fn only_the_live_bridge_uses_rumqttc() {
        fn walk(dir: &Path, offenders: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read src dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, offenders);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("bridge/live.rs")
                    && !path.ends_with("bridge/mod.rs")
                {
                    let text = std::fs::read_to_string(&path).expect("read source");
                    // This file's own assertions mention the name; look for real uses.
                    if text.contains("use rumqttc") || text.contains("rumqttc::") {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        let mut offenders = Vec::new();
        walk(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut offenders,
        );
        assert!(
            offenders.is_empty(),
            "outside the live bridge: {offenders:?}"
        );
    }
}
