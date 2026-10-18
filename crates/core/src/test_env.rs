//! Parameters for a test run of the app: where it keeps its data, which
//! background work it skips, and where it reads its update feed from.
//!
//! This module only *parses and resolves*: it decides what the flags mean and
//! needs no Tauri, so every rule is tested on any OS. Whether the app honours
//! them at all (debug builds and the `test-env` feature only) is decided in the
//! app crate -- a normal release never calls [`parse`].
//!
//! Every flag has an environment variable, `RESONANCE_TEST_<FLAG>` with the
//! dashes as underscores (`--no-capture` -> `RESONANCE_TEST_NO_CAPTURE`). A flag
//! on the command line wins over its variable.

use crate::update_signature::is_public_key;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Prefix of the environment variable of every flag.
pub const ENV_PREFIX: &str = "RESONANCE_TEST_";

/// The folders `--data-dir` makes: the three places the app writes.
const CONFIG_SUBDIR: &str = "config";
const DATA_SUBDIR: &str = "data";
const WEBVIEW_SUBDIR: &str = "webview";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// On or off; takes no value on the command line.
    Switch,
    /// Takes one value (`--flag value` or `--flag=value`).
    Value,
}

/// Every flag, in the order `TestEnv::set_flags` lists them.
const FLAGS: [(&str, Kind); 18] = [
    ("data-dir", Kind::Value),
    ("fresh", Kind::Switch),
    ("assume-setup-done", Kind::Switch),
    ("no-capture", Kind::Switch),
    ("no-translator", Kind::Switch),
    ("no-update-check", Kind::Switch),
    ("no-popups", Kind::Switch),
    ("no-window-state", Kind::Switch),
    ("feed-url", Kind::Value),
    ("metadata-url", Kind::Value),
    ("metadata-trust-key", Kind::Value),
    ("dictionary-url", Kind::Value),
    ("status-file", Kind::Value),
    ("log-file", Kind::Value),
    ("replay-chat", Kind::Value),
    ("bridge-url", Kind::Value),
    ("llama-url", Kind::Value),
    ("print-env", Kind::Switch),
];

/// What a test run asked for. `TestEnv::default()` is a normal run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestEnv {
    /// Root of everything the app writes (see [`resolve_dirs`]).
    pub data_dir: Option<PathBuf>,
    /// Empty `data_dir` first. Only valid together with `data_dir`.
    pub fresh: bool,
    /// Behave as if setup were finished, without writing that down.
    pub assume_setup_done: bool,
    pub no_capture: bool,
    pub no_translator: bool,
    pub no_update_check: bool,
    pub no_popups: bool,
    pub no_window_state: bool,
    /// Read the update feed from here instead of the release feed.
    pub feed_url: Option<String>,
    /// Read the gist metadata from here instead of the public gist.
    pub metadata_url: Option<String>,
    /// Trust this minisign public key (base64) for the signed metadata instead of the keys built into
    /// the app, so a mock server's throwaway key can sign it. Test builds only, like every flag.
    pub metadata_trust_key: Option<String>,
    /// Read the custom dictionary (`sync_dictionary`) from here instead of the public gist.
    pub dictionary_url: Option<String>,
    /// Write a JSON status file (start, ready, update state) here.
    pub status_file: Option<PathBuf>,
    /// Also write the log here (a release exe has no console).
    pub log_file: Option<PathBuf>,
    /// Feed the chat lines of this JSON Lines file in as if captured (see
    /// [`crate::replay`]). Not passed on after an update restart.
    pub replay_chat: Option<PathBuf>,
    /// Connect to this local MQTT broker (`mqtt://127.0.0.1:1883`) and publish
    /// what the app does / obey what the test publishes (see [`bridge_addr`]).
    pub bridge_url: Option<String>,
    /// Use the llama-server that answers here (`http://127.0.0.1:PORT`) instead of starting one: a stand-in
    /// that needs no model and no GPU. Local http only, with a port; stored without a trailing slash.
    pub llama_url: Option<String>,
    /// Print the resolved settings as JSON and exit.
    pub print_env: bool,
}

impl TestEnv {
    /// The flags that are set, by name, in a fixed order -- what the status
    /// file and `--print-env` report as "what was asked for".
    pub fn set_flags(&self) -> Vec<&'static str> {
        let set = [
            self.data_dir.is_some(),
            self.fresh,
            self.assume_setup_done,
            self.no_capture,
            self.no_translator,
            self.no_update_check,
            self.no_popups,
            self.no_window_state,
            self.feed_url.is_some(),
            self.metadata_url.is_some(),
            self.metadata_trust_key.is_some(),
            self.dictionary_url.is_some(),
            self.status_file.is_some(),
            self.log_file.is_some(),
            self.replay_chat.is_some(),
            self.bridge_url.is_some(),
            self.llama_url.is_some(),
            self.print_env,
        ];
        FLAGS
            .iter()
            .zip(set)
            .filter_map(|((name, _), on)| on.then_some(*name))
            .collect()
    }

    fn set_switch(&mut self, name: &str, on: bool) {
        match name {
            "fresh" => self.fresh = on,
            "assume-setup-done" => self.assume_setup_done = on,
            "no-capture" => self.no_capture = on,
            "no-translator" => self.no_translator = on,
            "no-update-check" => self.no_update_check = on,
            "no-popups" => self.no_popups = on,
            "no-window-state" => self.no_window_state = on,
            "print-env" => self.print_env = on,
            _ => unreachable!("not a switch: {name}"),
        }
    }

    fn set_value(&mut self, name: &str, value: String) -> Result<(), TestEnvError> {
        match name {
            "data-dir" => self.data_dir = Some(PathBuf::from(value)),
            "status-file" => self.status_file = Some(PathBuf::from(value)),
            "log-file" => self.log_file = Some(PathBuf::from(value)),
            "replay-chat" => self.replay_chat = Some(PathBuf::from(value)),
            "bridge-url" => {
                if bridge_addr(&value).is_none() {
                    return Err(TestEnvError::BadBridgeUrl { url: value });
                }
                self.bridge_url = Some(value);
            }
            "llama-url" => {
                let base = value.trim_end_matches('/').to_string();
                // Local http with a port, and no path: the app appends /health and /completion itself.
                let ok = is_local_http_url(&base)
                    && split_authority(&base).is_some_and(|(_, _, port)| port.is_some())
                    && base.matches('/').count() == 2;
                if !ok {
                    return Err(TestEnvError::BadLlamaUrl { url: value });
                }
                self.llama_url = Some(base);
            }
            "metadata-trust-key" => {
                if !is_public_key(&value) {
                    return Err(TestEnvError::BadTrustKey { key: value });
                }
                self.metadata_trust_key = Some(value);
            }
            "feed-url" | "metadata-url" | "dictionary-url" => {
                if !is_test_url_allowed(&value) {
                    return Err(TestEnvError::BadUrl {
                        flag: name.to_string(),
                        url: value,
                    });
                }
                match name {
                    "feed-url" => self.feed_url = Some(value),
                    "metadata-url" => self.metadata_url = Some(value),
                    _ => self.dictionary_url = Some(value),
                }
            }
            _ => unreachable!("not a value flag: {name}"),
        }
        Ok(())
    }
}

/// Why the parameters were refused. The app prints it and exits: a test run
/// that silently ignored a typo would test the wrong thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestEnvError {
    /// `--something` that is not a flag.
    UnknownFlag(String),
    /// A word that is not a flag at all.
    UnexpectedArg(String),
    /// A value flag at the end of the line, before another flag, or empty.
    MissingValue(String),
    /// `--fresh=1`: switches take no value (set `RESONANCE_TEST_FRESH=0` to turn one off).
    UnexpectedValue(String),
    /// An environment variable of a switch that is not yes/no.
    BadSwitchEnv { var: String, value: String },
    /// `--fresh` without `--data-dir` would empty the real data folders.
    FreshNeedsDataDir,
    /// A bridge URL that is not `mqtt://` to this machine.
    BadBridgeUrl { url: String },
    /// A feed / metadata URL that is neither `https` nor local `http`.
    BadUrl { flag: String, url: String },
    /// A `--metadata-trust-key` that is not a base64 minisign public key.
    BadTrustKey { key: String },
    /// A `--llama-url` that is not `http://` to this machine with a port.
    BadLlamaUrl { url: String },
}

impl fmt::Display for TestEnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(f, "unknown test flag --{flag}"),
            Self::UnexpectedArg(arg) => write!(f, "unexpected argument {arg:?}"),
            Self::MissingValue(flag) => write!(f, "--{flag} needs a value"),
            Self::UnexpectedValue(flag) => write!(f, "--{flag} takes no value"),
            Self::BadSwitchEnv { var, value } => {
                write!(
                    f,
                    "{var}={value:?} is not one of 1/0, true/false, yes/no, on/off"
                )
            }
            Self::FreshNeedsDataDir => {
                write!(
                    f,
                    "--fresh needs --data-dir (it would empty the real data folders)"
                )
            }
            Self::BadBridgeUrl { url } => write!(
                f,
                "--bridge-url {url:?}: only mqtt://127.0.0.1 / localhost / [::1], with an optional port"
            ),
            Self::BadLlamaUrl { url } => write!(
                f,
                "--llama-url {url:?}: only http://127.0.0.1:PORT / localhost / [::1], with a port and no path"
            ),
            Self::BadTrustKey { key } => write!(
                f,
                "--metadata-trust-key {key:?}: not a base64 minisign public key (the text of a .pub file from `tauri signer`)"
            ),
            Self::BadUrl { flag, url } => write!(
                f,
                "--{flag} {url:?}: only https:// or http://127.0.0.1 / localhost / [::1]"
            ),
        }
    }
}

impl std::error::Error for TestEnvError {}

fn env_name(flag: &str) -> String {
    format!("{ENV_PREFIX}{}", flag.to_uppercase().replace('-', "_"))
}

fn parse_switch_env(var: &str, value: &str) -> Result<bool, TestEnvError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "" | "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(TestEnvError::BadSwitchEnv {
            var: var.to_string(),
            value: value.to_string(),
        }),
    }
}

/// Reads the test parameters from the command line (without the program name)
/// and the environment (`env` looks one variable up).
pub fn parse<I, S>(args: I, env: impl Fn(&str) -> Option<String>) -> Result<TestEnv, TestEnvError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = TestEnv::default();

    for (name, kind) in FLAGS {
        let var = env_name(name);
        let Some(value) = env(&var) else { continue };
        match kind {
            Kind::Switch => out.set_switch(name, parse_switch_env(&var, &value)?),
            // An empty variable is "not set", as in every shell.
            Kind::Value if value.is_empty() => {}
            Kind::Value => out.set_value(name, value)?,
        }
    }

    let args: Vec<String> = args.into_iter().map(|a| a.as_ref().to_string()).collect();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        let Some(flag) = arg.strip_prefix("--") else {
            return Err(TestEnvError::UnexpectedArg(arg));
        };
        let (name, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (flag, None),
        };
        let Some(&(name, kind)) = FLAGS.iter().find(|(known, _)| *known == name) else {
            return Err(TestEnvError::UnknownFlag(name.to_string()));
        };
        match kind {
            Kind::Switch => {
                if inline.is_some() {
                    return Err(TestEnvError::UnexpectedValue(name.to_string()));
                }
                out.set_switch(name, true);
            }
            Kind::Value => {
                let value = match inline {
                    Some(value) => value,
                    None => match args.peek() {
                        Some(next) if !next.starts_with("--") => args.next().unwrap_or_default(),
                        _ => String::new(),
                    },
                };
                if value.is_empty() {
                    return Err(TestEnvError::MissingValue(name.to_string()));
                }
                out.set_value(name, value)?;
            }
        }
    }

    if out.fresh && out.data_dir.is_none() {
        return Err(TestEnvError::FreshNeedsDataDir);
    }
    Ok(out)
}

/// Whether `url` may be a test feed / metadata source: `https://host...`, or
/// plain `http` to this machine only (see [`is_local_http_url`]), so a test can
/// run against a local mock server without ever sending an update check over
/// the network unencrypted.
pub fn is_test_url_allowed(url: &str) -> bool {
    match split_url(url) {
        Some((scheme, _)) if scheme == "https" => true,
        Some(_) => is_local_http_url(url),
        None => false,
    }
}

/// `http://` to this machine: `127.0.0.1`, `localhost` or `[::1]`, with an
/// optional numeric port. Nothing else counts -- not `https`, not another host,
/// not a host that merely starts like one (`127.0.0.1.example.com`), not a URL
/// with a user name (`http://127.0.0.1@example.com/` is a request to example.com).
pub fn is_local_http_url(url: &str) -> bool {
    match split_url(url) {
        Some((scheme, host)) if scheme == "http" => {
            matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
        }
        _ => false,
    }
}

/// (lowercase scheme, lowercase host) of a URL whose authority is well formed
/// (no user name, a host, a numeric port if any); `None` otherwise.
fn split_url(url: &str) -> Option<(String, String)> {
    split_authority(url).map(|(scheme, host, _)| (scheme, host))
}

/// Like [`split_url`], and the port as written (`None` when there is none).
fn split_authority(url: &str) -> Option<(String, String, Option<String>)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return None;
    }
    let (host, port) = host_and_port(authority)?;
    if !port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        port.map(String::from),
    ))
}

/// Default port of an MQTT broker.
pub const BRIDGE_DEFAULT_PORT: u16 = 1883;

/// (host, port) of a `--bridge-url`: `mqtt://` to this machine only
/// (`127.0.0.1`, `localhost` or `[::1]`), port 1-65535, 1883 when left out. The
/// bridge carries chat text, so it never leaves the machine.
pub fn bridge_addr(url: &str) -> Option<(String, u16)> {
    let (scheme, host, port) = split_authority(url)?;
    if scheme != "mqtt" || !matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
        return None;
    }
    let port = match port {
        None => BRIDGE_DEFAULT_PORT,
        Some(p) => p.parse::<u16>().ok().filter(|&p| p != 0)?,
    };
    Some((host, port))
}

/// `host`, `host:port`, `[v6]` or `[v6]:port` -> (host, port). `None` when the
/// host is empty or the brackets do not match.
fn host_and_port(authority: &str) -> Option<(&str, Option<&str>)> {
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']')? + 1;
        let (host, rest) = authority.split_at(end);
        match rest {
            "" => (host, None),
            _ => (host, Some(rest.strip_prefix(':')?)),
        }
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    (!host.is_empty() && host != "[]").then_some((host, port))
}

/// The folders the app writes to in this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// The `--data-dir` these were made from; `None` in a normal run.
    pub root: Option<PathBuf>,
    /// `config.json`, metadata (Tauri's `app_config_dir`).
    pub config: PathBuf,
    /// Model, server, chat logs, dictionary, raw capture (Tauri's `app_data_dir`).
    pub data: PathBuf,
    /// WebView2's user data folder to set (`WEBVIEW2_USER_DATA_FOLDER`);
    /// `None` leaves the default under `%LOCALAPPDATA%`.
    pub webview: Option<PathBuf>,
}

/// The folders for this run: the app's own defaults, or the three folders
/// under `--data-dir`. With no flag the defaults come back unchanged.
pub fn resolve_dirs(default_config: &Path, default_data: &Path, env: &TestEnv) -> AppDirs {
    match &env.data_dir {
        Some(root) => AppDirs {
            root: Some(root.clone()),
            config: root.join(CONFIG_SUBDIR),
            data: root.join(DATA_SUBDIR),
            webview: Some(root.join(WEBVIEW_SUBDIR)),
        },
        None => AppDirs {
            root: None,
            config: default_config.to_path_buf(),
            data: default_data.to_path_buf(),
            webview: None,
        },
    }
}

impl AppDirs {
    /// `--fresh`: empties the three folders under the root and creates them
    /// again. Nothing else in the root is touched, and a root that looks like a
    /// drive or a top-level folder (`C:\`, `/`, `/home`) or has a `..` is
    /// refused, so a wrong `--data-dir` cannot cost more than the test's own
    /// folders. A run without a root has nothing to reset: that is an error.
    pub fn reset(&self) -> io::Result<()> {
        let invalid = |msg: &str| io::Error::new(io::ErrorKind::InvalidInput, msg.to_string());
        let root = self
            .root
            .as_deref()
            .ok_or_else(|| invalid("no --data-dir: nothing to reset"))?;
        if root.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(invalid("--data-dir must not contain .."));
        }
        // Drive/root + at least two names: `C:\w1\run1`, `/tmp/run1`.
        if root.components().count() < 3 {
            return Err(invalid("--data-dir is too close to the top of a drive"));
        }
        for dir in [Some(&self.config), Some(&self.data), self.webview.as_ref()]
            .into_iter()
            .flatten()
        {
            match std::fs::remove_dir_all(dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

impl TestEnv {
    /// Is setup finished for this run? `stored` is what `config.json` says;
    /// `--assume-setup-done` makes it true either way.
    pub fn init_done_for_run(&self, stored: bool) -> bool {
        stored || self.assume_setup_done
    }

    /// What to write to `config.json` as `init_done`: `current` normally; with
    /// `--assume-setup-done`, what the file held (`stored`), so the flag is
    /// never written down.
    pub fn init_done_for_disk(&self, current: bool, stored: bool) -> bool {
        if self.assume_setup_done {
            stored
        } else {
            current
        }
    }
}

/// The flags a restarted app (after an update) gets, so the new process runs
/// in the same test environment. `--fresh` would empty the folders again and
/// `--print-env` would exit at once, so neither is passed on.
impl TestEnv {
    pub fn restart_args(&self) -> Vec<String> {
        let switch = |on: bool, name: &str| on.then(|| format!("--{name}"));
        let value = |value: Option<String>, name: &str| value.map(|v| format!("--{name}={v}"));
        let path = |path: &Option<PathBuf>| path.as_ref().map(|p| p.to_string_lossy().into_owned());
        // The order of `FLAGS`, so the same settings always give the same line.
        [
            value(path(&self.data_dir), "data-dir"),
            switch(self.assume_setup_done, "assume-setup-done"),
            switch(self.no_capture, "no-capture"),
            switch(self.no_translator, "no-translator"),
            switch(self.no_update_check, "no-update-check"),
            switch(self.no_popups, "no-popups"),
            switch(self.no_window_state, "no-window-state"),
            value(self.feed_url.clone(), "feed-url"),
            value(self.metadata_url.clone(), "metadata-url"),
            value(self.metadata_trust_key.clone(), "metadata-trust-key"),
            value(self.dictionary_url.clone(), "dictionary-url"),
            value(path(&self.status_file), "status-file"),
            value(path(&self.log_file), "log-file"),
            value(self.bridge_url.clone(), "bridge-url"),
            value(self.llama_url.clone(), "llama-url"),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// How the app's own update stands, as a test script reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// No check yet, or nothing newer.
    None,
    Available(String),
    Downloading,
    Downloaded,
    Error(String),
}

impl UpdateState {
    /// The state a finished update check leaves: the error that stopped it, else
    /// the newer version it found, else nothing.
    pub fn after_check(feed_error: Option<&str>, newer_version: Option<&str>) -> Self {
        match (feed_error, newer_version) {
            (Some(error), _) => Self::Error(error.to_string()),
            (None, Some(version)) => Self::Available(version.to_string()),
            (None, None) => Self::None,
        }
    }
}

impl fmt::Display for UpdateState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Available(version) => write!(f, "available:{version}"),
            Self::Downloading => write!(f, "downloading"),
            Self::Downloaded => write!(f, "downloaded"),
            Self::Error(reason) => {
                // One line, so a script can read it back from the file.
                let reason: Vec<&str> = reason.split_whitespace().collect();
                write!(f, "error:{}", reason.join(" "))
            }
        }
    }
}

/// What `--status-file` and `--print-env` report. The keys are stable: a test
/// script reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub version: String,
    /// Full path of the running exe -- which copy is this?
    pub exe: String,
    pub pid: u32,
    /// The app has finished starting.
    pub ready: bool,
    /// `None` while the folder is still Tauri's default and not yet resolved.
    pub config_dir: Option<String>,
    pub data_dir: Option<String>,
    /// Names of the flags that are set (`TestEnv::set_flags`).
    pub flags: Vec<String>,
    pub update: UpdateState,
}

impl StatusReport {
    pub fn to_json(&self) -> String {
        let json = serde_json::json!({
            "version": self.version,
            "exe": self.exe,
            "pid": self.pid,
            "ready": self.ready,
            "config_dir": self.config_dir,
            "data_dir": self.data_dir,
            "flags": self.flags,
            "update": self.update.to_string(),
        });
        serde_json::to_string_pretty(&json).unwrap_or_default()
    }

    /// Writes the JSON to `path`, replacing the file in one step so a script
    /// polling it never reads half of one.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        crate::download::write_atomic(path, self.to_json().as_bytes())
    }
}

/// Writes everything to both `A` and `B`. An error from `B` (a log file that
/// cannot be written) is ignored: the log must never take the app down.
pub struct Tee<A: io::Write, B: io::Write>(pub A, pub B);

impl<A: io::Write, B: io::Write> io::Write for Tee<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.0.write(buf)?;
        let _ = self.1.write_all(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.1.flush();
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn run(args: &[&str], vars: &[(&str, &str)]) -> Result<TestEnv, TestEnvError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        parse(args.iter().copied(), |k| vars.get(k).cloned())
    }

    fn ok(args: &[&str]) -> TestEnv {
        run(args, &[]).expect("parses")
    }

    #[test]
    fn no_flags_is_a_normal_run() {
        let env = ok(&[]);
        assert_eq!(env, TestEnv::default());
        assert!(env.set_flags().is_empty());
    }

    #[test]
    fn a_value_flag_takes_the_next_word_or_an_equals_sign() {
        assert_eq!(
            ok(&["--data-dir", "C:\\w1\\run1"]).data_dir,
            Some("C:\\w1\\run1".into())
        );
        assert_eq!(
            ok(&["--data-dir=/tmp/run1"]).data_dir,
            Some("/tmp/run1".into())
        );
        // A value may itself contain `=`.
        assert_eq!(
            ok(&["--log-file=/tmp/a=b.log"]).log_file,
            Some("/tmp/a=b.log".into())
        );
    }

    #[test]
    fn every_switch_turns_on() {
        let env = ok(&[
            "--assume-setup-done",
            "--no-capture",
            "--no-translator",
            "--no-update-check",
            "--no-popups",
            "--no-window-state",
            "--print-env",
        ]);
        assert!(env.assume_setup_done);
        assert!(env.no_capture);
        assert!(env.no_translator);
        assert!(env.no_update_check);
        assert!(env.no_popups);
        assert!(env.no_window_state);
        assert!(env.print_env);
        assert!(!env.fresh);
    }

    #[test]
    fn the_other_value_flags_are_read() {
        let env = ok(&[
            "--status-file",
            "/tmp/status.json",
            "--log-file",
            "/tmp/app.log",
            "--feed-url",
            "https://example.com/latest.json",
            "--metadata-url=http://127.0.0.1:8099/meta.json",
            "--dictionary-url=http://127.0.0.1:8099/custom_dict.json",
        ]);
        assert_eq!(env.status_file, Some("/tmp/status.json".into()));
        assert_eq!(env.log_file, Some("/tmp/app.log".into()));
        assert_eq!(
            env.feed_url.as_deref(),
            Some("https://example.com/latest.json")
        );
        assert_eq!(
            env.metadata_url.as_deref(),
            Some("http://127.0.0.1:8099/meta.json")
        );
        assert_eq!(
            env.dictionary_url.as_deref(),
            Some("http://127.0.0.1:8099/custom_dict.json")
        );
    }

    #[test]
    fn a_dictionary_url_is_https_or_local_http_and_is_passed_on_a_restart() {
        // The same rule as the feed and metadata URLs: a test must not be able to send the app to some other host.
        assert_eq!(
            run(
                &["--dictionary-url", "http://example.com/custom_dict.json"],
                &[]
            ),
            Err(TestEnvError::BadUrl {
                flag: "dictionary-url".into(),
                url: "http://example.com/custom_dict.json".into()
            })
        );
        let env = ok(&["--dictionary-url", "https://example.com/custom_dict.json"]);
        assert_eq!(env.set_flags(), ["dictionary-url"]);
        assert_eq!(
            env.restart_args(),
            ["--dictionary-url=https://example.com/custom_dict.json"]
        );
    }

    // A throwaway public key (see signed_metadata's tests): it signs nothing real.
    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDEyM0EzNEMxRkU1RjEzNTUKUldSVkUxLyt3VFE2RWxFcjNsMzhMVGJIekpzTDMwVWlDZktzM3VQelZZS2o4eHJlcWpXajRCUncK";

    #[test]
    fn a_metadata_trust_key_is_a_public_key_and_is_passed_on_a_restart() {
        let env = ok(&["--metadata-trust-key", TEST_KEY]);
        assert_eq!(env.metadata_trust_key.as_deref(), Some(TEST_KEY));
        assert_eq!(env.set_flags(), ["metadata-trust-key"]);
        assert_eq!(
            env.restart_args(),
            [format!("--metadata-trust-key={TEST_KEY}")]
        );
        // The variable works like every other value flag.
        let env = run(&[], &[("RESONANCE_TEST_METADATA_TRUST_KEY", TEST_KEY)]).expect("parses");
        assert_eq!(env.metadata_trust_key.as_deref(), Some(TEST_KEY));
        // A typo must not silently leave the real keys in charge of the wrong test.
        assert_eq!(
            run(&["--metadata-trust-key", "not-a-key"], &[]),
            Err(TestEnvError::BadTrustKey {
                key: "not-a-key".into()
            })
        );
        assert_eq!(
            run(&["--metadata-trust-key"], &[]),
            Err(TestEnvError::MissingValue("metadata-trust-key".into()))
        );
    }

    #[test]
    fn the_trust_key_comes_after_the_metadata_url_in_the_flag_order() {
        let env = ok(&[
            "--metadata-trust-key",
            TEST_KEY,
            "--dictionary-url",
            "https://example.com/d.json",
            "--metadata-url",
            "https://example.com/m.json",
        ]);
        assert_eq!(
            env.set_flags(),
            ["metadata-url", "metadata-trust-key", "dictionary-url"]
        );
        assert_eq!(
            env.restart_args(),
            [
                "--metadata-url=https://example.com/m.json".to_string(),
                format!("--metadata-trust-key={TEST_KEY}"),
                "--dictionary-url=https://example.com/d.json".to_string(),
            ]
        );
    }

    #[test]
    fn an_unknown_flag_or_a_stray_word_is_an_error() {
        assert_eq!(
            run(&["--no-captur"], &[]),
            Err(TestEnvError::UnknownFlag("no-captur".into()))
        );
        assert_eq!(
            run(&["--bogus=1"], &[]),
            Err(TestEnvError::UnknownFlag("bogus".into()))
        );
        assert_eq!(
            run(&["data-dir"], &[]),
            Err(TestEnvError::UnexpectedArg("data-dir".into()))
        );
    }

    #[test]
    fn a_value_flag_without_a_value_is_an_error() {
        let missing = |flag: &str| Err(TestEnvError::MissingValue(flag.into()));
        assert_eq!(run(&["--data-dir"], &[]), missing("data-dir"));
        // The next word is a flag, not the value.
        assert_eq!(run(&["--data-dir", "--fresh"], &[]), missing("data-dir"));
        assert_eq!(run(&["--log-file="], &[]), missing("log-file"));
    }

    #[test]
    fn a_switch_given_a_value_is_an_error() {
        assert_eq!(
            run(&["--no-capture=1"], &[]),
            Err(TestEnvError::UnexpectedValue("no-capture".into()))
        );
    }

    #[test]
    fn the_environment_sets_flags() {
        let env = run(
            &[],
            &[
                ("RESONANCE_TEST_DATA_DIR", "/tmp/run1"),
                ("RESONANCE_TEST_NO_CAPTURE", "1"),
                ("RESONANCE_TEST_ASSUME_SETUP_DONE", "TRUE"),
                ("RESONANCE_TEST_NO_POPUPS", "0"),
                ("RESONANCE_TEST_FEED_URL", "http://localhost:9/f.json"),
            ],
        )
        .expect("parses");
        assert_eq!(env.data_dir, Some("/tmp/run1".into()));
        assert!(env.no_capture);
        assert!(env.assume_setup_done);
        assert!(!env.no_popups);
        assert_eq!(env.feed_url.as_deref(), Some("http://localhost:9/f.json"));
    }

    #[test]
    fn an_empty_variable_is_not_set() {
        let env = run(
            &[],
            &[
                ("RESONANCE_TEST_DATA_DIR", ""),
                ("RESONANCE_TEST_NO_CAPTURE", ""),
            ],
        )
        .expect("parses");
        assert_eq!(env, TestEnv::default());
    }

    #[test]
    fn a_switch_variable_that_is_not_yes_or_no_is_an_error() {
        assert_eq!(
            run(&[], &[("RESONANCE_TEST_NO_CAPTURE", "maybe")]),
            Err(TestEnvError::BadSwitchEnv {
                var: "RESONANCE_TEST_NO_CAPTURE".into(),
                value: "maybe".into()
            })
        );
    }

    #[test]
    fn a_flag_wins_over_its_variable() {
        let env = run(
            &["--data-dir", "/tmp/from-flag", "--no-capture"],
            &[
                ("RESONANCE_TEST_DATA_DIR", "/tmp/from-env"),
                ("RESONANCE_TEST_NO_CAPTURE", "0"),
            ],
        )
        .expect("parses");
        assert_eq!(env.data_dir, Some("/tmp/from-flag".into()));
        assert!(env.no_capture);
    }

    #[test]
    fn fresh_needs_a_data_dir_from_either_source() {
        assert_eq!(run(&["--fresh"], &[]), Err(TestEnvError::FreshNeedsDataDir));
        assert_eq!(
            run(&[], &[("RESONANCE_TEST_FRESH", "1")]),
            Err(TestEnvError::FreshNeedsDataDir)
        );
        assert!(ok(&["--fresh", "--data-dir", "/tmp/run1"]).fresh);
        let env = run(&["--fresh"], &[("RESONANCE_TEST_DATA_DIR", "/tmp/run1")]).expect("parses");
        assert!(env.fresh);
    }

    #[test]
    fn a_bad_feed_url_is_an_error_from_a_flag_and_from_the_variable() {
        let bad = Err(TestEnvError::BadUrl {
            flag: "feed-url".into(),
            url: "http://example.com/latest.json".into(),
        });
        assert_eq!(
            run(&["--feed-url", "http://example.com/latest.json"], &[]),
            bad
        );
        assert_eq!(
            run(
                &[],
                &[("RESONANCE_TEST_FEED_URL", "http://example.com/latest.json")]
            ),
            bad
        );
    }

    #[test]
    fn set_flags_lists_what_is_set_in_a_fixed_order() {
        let env = ok(&[
            "--print-env",
            "--no-capture",
            "--data-dir",
            "/tmp/run1",
            "--fresh",
        ]);
        assert_eq!(
            env.set_flags(),
            ["data-dir", "fresh", "no-capture", "print-env"]
        );
    }

    #[test]
    fn https_and_local_http_urls_are_allowed() {
        for url in [
            "https://example.com/latest.json",
            "HTTPS://example.com",
            "https://example.com:8443/a?b=c#d",
            "http://127.0.0.1/feed",
            "http://127.0.0.1:8099/feed.json",
            "http://localhost:8099",
            "http://LOCALHOST/feed",
            "http://[::1]:8099/feed",
            "http://[::1]/feed",
        ] {
            assert!(is_test_url_allowed(url), "{url} should be allowed");
        }
    }

    #[test]
    fn bridge_url_is_a_local_mqtt_broker_from_the_flag_or_the_variable() {
        let env = ok(&["--bridge-url", "mqtt://127.0.0.1:1884"]);
        assert_eq!(env.bridge_url.as_deref(), Some("mqtt://127.0.0.1:1884"));
        assert!(env.set_flags().contains(&"bridge-url"));
        let env = run(&[], &[("RESONANCE_TEST_BRIDGE_URL", "mqtt://localhost")]).expect("parses");
        assert_eq!(env.bridge_url.as_deref(), Some("mqtt://localhost"));
        assert_eq!(
            run(&["--bridge-url"], &[]),
            Err(TestEnvError::MissingValue("bridge-url".into()))
        );
    }

    #[test]
    fn llama_url_is_a_local_http_server_with_a_port() {
        let env = ok(&["--llama-url", "http://127.0.0.1:8099"]);
        assert_eq!(env.llama_url.as_deref(), Some("http://127.0.0.1:8099"));
        assert!(env.set_flags().contains(&"llama-url"));
        // A trailing slash is not part of the base address the app appends /completion to.
        let env = ok(&["--llama-url=http://localhost:8099/"]);
        assert_eq!(env.llama_url.as_deref(), Some("http://localhost:8099"));
        let env = run(&[], &[("RESONANCE_TEST_LLAMA_URL", "http://[::1]:9")]).expect("parses");
        assert_eq!(env.llama_url.as_deref(), Some("http://[::1]:9"));
        assert_eq!(
            run(&["--llama-url"], &[]),
            Err(TestEnvError::MissingValue("llama-url".into()))
        );
        // A restarted app keeps talking to the same stand-in.
        assert_eq!(
            ok(&["--llama-url", "http://127.0.0.1:8099"]).restart_args(),
            ["--llama-url=http://127.0.0.1:8099"]
        );
    }

    #[test]
    fn devtools_port_is_a_port_from_the_flag_or_the_variable() {
        let env = ok(&["--devtools-port", "9222"]);
        assert_eq!(env.devtools_port, Some(9222));
        assert!(env.set_flags().contains(&"devtools-port"));
        let env = run(&[], &[("RESONANCE_TEST_DEVTOOLS_PORT", "6003")]).expect("parses");
        assert_eq!(env.devtools_port, Some(6003));
        for value in ["0", "70000", "abc", "-1", "9222x"] {
            assert_eq!(
                run(&["--devtools-port", value], &[]),
                Err(TestEnvError::BadDevtoolsPort {
                    value: value.into()
                }),
                "{value}"
            );
        }
        assert_eq!(
            run(&["--devtools-port"], &[]),
            Err(TestEnvError::MissingValue("devtools-port".into()))
        );
    }

    #[test]
    fn a_restart_does_not_open_the_devtools_port_again() {
        // The new process starts while the old one still holds the port.
        let env = ok(&["--data-dir", "/tmp/run1", "--devtools-port", "9222"]);
        assert_eq!(env.restart_args(), ["--data-dir=/tmp/run1"]);
    }

    #[test]
    fn llama_url_refuses_anything_but_local_http_with_a_port() {
        for url in [
            "http://llama.example.com:8080",
            "https://127.0.0.1:8080",
            "http://127.0.0.1",
            "http://127.0.0.1:abc",
            "http://127.0.0.1.example.com:8080",
            "http://user@127.0.0.1:8080",
            "http://127.0.0.1:8080/v1/x",
            "127.0.0.1:8080",
        ] {
            assert_eq!(
                run(&["--llama-url", url], &[]),
                Err(TestEnvError::BadLlamaUrl { url: url.into() }),
                "{url}"
            );
        }
    }

    #[test]
    fn bridge_url_refuses_anything_but_mqtt_to_this_machine() {
        for url in [
            "mqtt://broker.example.com:1883",
            "mqtt://127.0.0.1.example.com",
            "mqtt://user@127.0.0.1:1883",
            "mqtts://127.0.0.1:8883",
            "http://127.0.0.1:1883",
            "mqtt://127.0.0.1:0",
            "mqtt://127.0.0.1:70000",
            "mqtt://127.0.0.1:abc",
            "127.0.0.1:1883",
        ] {
            assert_eq!(
                run(&["--bridge-url", url], &[]),
                Err(TestEnvError::BadBridgeUrl { url: url.into() }),
                "{url}"
            );
        }
        assert_eq!(
            run(&[], &[("RESONANCE_TEST_BRIDGE_URL", "mqtt://example.com")]),
            Err(TestEnvError::BadBridgeUrl {
                url: "mqtt://example.com".into()
            })
        );
    }

    #[test]
    fn a_bridge_address_has_a_host_and_a_port_that_defaults_to_1883() {
        assert_eq!(
            bridge_addr("mqtt://127.0.0.1:1884"),
            Some(("127.0.0.1".to_string(), 1884))
        );
        assert_eq!(
            bridge_addr("mqtt://LOCALHOST"),
            Some(("localhost".to_string(), 1883))
        );
        assert_eq!(
            bridge_addr("mqtt://[::1]:9/"),
            Some(("[::1]".to_string(), 9))
        );
        assert_eq!(bridge_addr("mqtt://example.com"), None);
    }

    #[test]
    fn a_restart_keeps_the_bridge() {
        let env = ok(&["--bridge-url", "mqtt://127.0.0.1:1884"]);
        assert_eq!(env.restart_args(), ["--bridge-url=mqtt://127.0.0.1:1884"]);
    }

    #[test]
    fn replay_chat_takes_a_file_from_the_flag_or_the_variable() {
        assert_eq!(
            ok(&["--replay-chat", "C:\\w1\\chat.jsonl"]).replay_chat,
            Some("C:\\w1\\chat.jsonl".into())
        );
        let env = run(&[], &[("RESONANCE_TEST_REPLAY_CHAT", "/tmp/chat.jsonl")]).expect("parses");
        assert_eq!(env.replay_chat, Some("/tmp/chat.jsonl".into()));
        assert_eq!(
            run(&["--replay-chat"], &[]),
            Err(TestEnvError::MissingValue("replay-chat".into()))
        );
        assert!(ok(&["--replay-chat", "x.jsonl", "--no-capture"])
            .set_flags()
            .contains(&"replay-chat"));
    }

    #[test]
    fn a_restart_does_not_replay_the_chat_again() {
        let env = ok(&[
            "--data-dir",
            "/tmp/run1",
            "--replay-chat",
            "/tmp/chat.jsonl",
        ]);
        assert_eq!(env.restart_args(), ["--data-dir=/tmp/run1"]);
    }

    #[test]
    fn local_http_is_http_to_this_machine_only() {
        for url in [
            "http://127.0.0.1/feed",
            "http://127.0.0.1:8099/feed.json",
            "http://localhost:8099",
            "HTTP://LOCALHOST/feed",
            "http://[::1]:8099/feed",
        ] {
            assert!(is_local_http_url(url), "{url} should be local http");
        }
        for url in [
            "",
            "https://127.0.0.1/feed",
            "https://example.com/feed",
            "http://example.com/feed",
            "http://127.0.0.1.evil.example/feed",
            "http://127.0.0.1@evil.example/feed",
            "http://127.0.0.1:/feed",
            "ftp://127.0.0.1/feed",
        ] {
            assert!(!is_local_http_url(url), "{url} should not be local http");
        }
    }

    #[test]
    fn other_urls_are_refused() {
        for url in [
            "",
            "example.com/feed",
            "https://",
            "https:///feed",
            "http://example.com/feed",
            "http://127.0.0.1.evil.example/feed",
            "http://localhost.evil.example/feed",
            "http://127.0.0.1@evil.example/feed",
            "https://user:pw@example.com/feed",
            "http://127.0.0.1:/feed",
            "http://127.0.0.1:80x/feed",
            "http://[::1/feed",
            "http://[]/feed",
            "ftp://127.0.0.1/feed",
            "file:///C:/feed.json",
        ] {
            assert!(!is_test_url_allowed(url), "{url} should be refused");
        }
    }

    #[test]
    fn without_a_data_dir_the_defaults_come_back_unchanged() {
        let dirs = resolve_dirs(Path::new("/def/config"), Path::new("/def/data"), &ok(&[]));
        assert_eq!(
            dirs,
            AppDirs {
                root: None,
                config: "/def/config".into(),
                data: "/def/data".into(),
                webview: None,
            }
        );
    }

    #[test]
    fn a_data_dir_holds_config_data_and_webview() {
        let env = ok(&["--data-dir", "/tmp/run1"]);
        let dirs = resolve_dirs(Path::new("/def/config"), Path::new("/def/data"), &env);
        let root = PathBuf::from("/tmp/run1");
        assert_eq!(dirs.root, Some(root.clone()));
        assert_eq!(dirs.config, root.join("config"));
        assert_eq!(dirs.data, root.join("data"));
        assert_eq!(dirs.webview, Some(root.join("webview")));
    }

    /// A folder of its own under the OS temp dir, empty.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("resonance-test-env-{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn dirs_for(root: &Path) -> AppDirs {
        let env = TestEnv {
            data_dir: Some(root.to_path_buf()),
            ..TestEnv::default()
        };
        resolve_dirs(Path::new("/def/config"), Path::new("/def/data"), &env)
    }

    #[test]
    fn reset_empties_the_three_folders_and_leaves_the_rest_of_the_root() {
        let root = scratch("reset-keeps-rest");
        let dirs = dirs_for(&root);
        for dir in [&dirs.config, &dirs.data] {
            std::fs::create_dir_all(dir.join("sub")).unwrap();
            std::fs::write(dir.join("sub").join("old.txt"), "old").unwrap();
        }
        std::fs::write(root.join("keep.txt"), "keep").unwrap();

        dirs.reset().expect("reset");

        for dir in [Some(&dirs.config), Some(&dirs.data), dirs.webview.as_ref()]
            .into_iter()
            .flatten()
        {
            assert!(dir.is_dir(), "{dir:?} exists");
            assert_eq!(
                std::fs::read_dir(dir).unwrap().count(),
                0,
                "{dir:?} is empty"
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join("keep.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn reset_creates_the_folders_of_a_new_root() {
        let root = scratch("reset-new").join("not-yet");
        let dirs = dirs_for(&root);
        dirs.reset().expect("reset");
        assert!(dirs.config.is_dir() && dirs.data.is_dir());
        assert!(dirs.webview.as_ref().is_some_and(|w| w.is_dir()));
    }

    #[test]
    fn reset_refuses_a_root_near_the_top_or_with_parent_steps() {
        for root in ["/", "/home", "C:\\", "/tmp/../etc/x"] {
            let dirs = dirs_for(Path::new(root));
            assert!(dirs.reset().is_err(), "{root} should be refused");
        }
    }

    #[test]
    fn reset_without_a_data_dir_is_an_error() {
        let dirs = resolve_dirs(Path::new("/def/config"), Path::new("/def/data"), &ok(&[]));
        assert!(dirs.reset().is_err());
    }

    // ---- status report, restart arguments, log tee ----

    #[test]
    fn update_states_have_stable_names() {
        assert_eq!(UpdateState::None.to_string(), "none");
        assert_eq!(
            UpdateState::Available("0.6.2".into()).to_string(),
            "available:0.6.2"
        );
        assert_eq!(UpdateState::Downloading.to_string(), "downloading");
        assert_eq!(UpdateState::Downloaded.to_string(), "downloaded");
        assert_eq!(
            UpdateState::Error("Network error".into()).to_string(),
            "error:Network error"
        );
    }

    #[test]
    fn assume_setup_done_counts_for_the_run_but_is_never_written() {
        let normal = TestEnv::default();
        let assume = ok(&["--assume-setup-done"]);

        // For the run: set up if the file says so, or the flag does.
        assert!(!normal.init_done_for_run(false));
        assert!(normal.init_done_for_run(true));
        assert!(assume.init_done_for_run(false));
        assert!(assume.init_done_for_run(true));

        // On disk: a normal run writes what it has; an assuming run writes back
        // what was stored, so the flag leaves no trace in the file.
        assert!(normal.init_done_for_disk(true, false));
        assert!(!normal.init_done_for_disk(false, true));
        assert!(!assume.init_done_for_disk(true, false));
        assert!(assume.init_done_for_disk(false, true));
    }

    #[test]
    fn an_update_check_ends_in_error_available_or_none() {
        let after = UpdateState::after_check;
        assert_eq!(
            after(Some("Network error"), None),
            UpdateState::Error("Network error".into())
        );
        // An error wins: the feed could not be read, whatever was known before.
        assert_eq!(
            after(Some("Network error"), Some("0.6.2")),
            UpdateState::Error("Network error".into())
        );
        assert_eq!(
            after(None, Some("0.6.2")),
            UpdateState::Available("0.6.2".into())
        );
        assert_eq!(after(None, None), UpdateState::None);
    }

    #[test]
    fn an_error_reason_stays_on_one_line() {
        assert_eq!(
            UpdateState::Error("bad\r\nfeed\n".into()).to_string(),
            "error:bad feed"
        );
    }

    fn report() -> StatusReport {
        StatusReport {
            version: "0.6.1".into(),
            exe: "C:\\w1\\app.exe".into(),
            pid: 4242,
            ready: false,
            config_dir: None,
            data_dir: Some("/tmp/run1/data".into()),
            flags: vec!["data-dir".into(), "no-capture".into()],
            update: UpdateState::Available("0.6.2".into()),
        }
    }

    #[test]
    fn the_status_json_has_stable_keys() {
        let json: serde_json::Value = serde_json::from_str(&report().to_json()).expect("json");
        assert_eq!(json["version"], "0.6.1");
        assert_eq!(json["exe"], "C:\\w1\\app.exe");
        assert_eq!(json["pid"], 4242);
        assert_eq!(json["ready"], false);
        assert!(json["config_dir"].is_null());
        assert_eq!(json["data_dir"], "/tmp/run1/data");
        assert_eq!(json["flags"], serde_json::json!(["data-dir", "no-capture"]));
        assert_eq!(json["update"], "available:0.6.2");
        assert_eq!(json.as_object().expect("object").len(), 8);
    }

    #[test]
    fn writing_a_status_file_replaces_the_old_one() {
        let dir = scratch("status-file");
        let path = dir.join("nested").join("status.json");
        report().write(&path).expect("first write");
        let mut ready = report();
        ready.ready = true;
        ready.write(&path).expect("second write");

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["ready"], true);
        let files: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(files.len(), 1, "no temp file is left behind");
    }

    #[test]
    fn a_restart_gets_the_same_flags_except_fresh_and_print_env() {
        assert!(TestEnv::default().restart_args().is_empty());

        let env = ok(&[
            "--data-dir",
            "/tmp/run1",
            "--fresh",
            "--print-env",
            "--no-capture",
            "--feed-url",
            "http://127.0.0.1:8099/latest.json",
        ]);
        let args = env.restart_args();
        assert_eq!(
            args,
            [
                "--data-dir=/tmp/run1",
                "--no-capture",
                "--feed-url=http://127.0.0.1:8099/latest.json"
            ]
        );
        // And they mean the same thing when read back.
        let again = ok(&args.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(
            again,
            TestEnv {
                fresh: false,
                print_env: false,
                ..env
            }
        );
    }

    /// A writer that always fails, as a full disk or a closed file would.
    struct Broken;
    impl io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("broken"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("broken"))
        }
    }

    #[test]
    fn a_tee_writes_to_both_sides() {
        let mut tee = Tee(Vec::new(), Vec::new());
        io::Write::write_all(&mut tee, b"hello").unwrap();
        io::Write::flush(&mut tee).unwrap();
        assert_eq!(tee.0, b"hello");
        assert_eq!(tee.1, b"hello");
    }

    #[test]
    fn a_broken_log_file_never_breaks_the_console_log() {
        let mut tee = Tee(Vec::new(), Broken);
        io::Write::write_all(&mut tee, b"hello").unwrap();
        io::Write::flush(&mut tee).unwrap();
        assert_eq!(tee.0, b"hello");
    }
}
