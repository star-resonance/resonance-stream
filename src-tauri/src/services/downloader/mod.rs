mod app_updater;
mod fetch;
mod gist;
pub mod model;
pub mod server;

pub use self::app_updater::*;
pub use self::gist::*;
pub use self::model::*;
pub use self::server::*;
pub use resonance_types::{FolderStatus, ProgressPayload};

use parking_lot::Mutex;
use resonance_core::download::SingleRun;
use resonance_core::update_feed::UpdateFeed;

/// What the update checks and the app update download know between calls. Owned by `AppState`.
#[derive(Debug, Default)]
pub struct Updates {
    /// The release the last update check announced. The app updater downloads and verifies
    /// exactly this one: its url, its version and its signature.
    announced: Mutex<Option<UpdateFeed>>,
    /// The signed metadata the last check accepted. `sync_dictionary` checks the dictionary
    /// against it, so the UI cannot name another version or hash.
    verified: Mutex<Option<GistMetadata>>,
    /// The release whose exe is sitting in `update_temp.exe`, once it has passed the signature
    /// check. `restart_to_apply_update` checks the file again against it right before the swap.
    downloaded: Mutex<Option<UpdateFeed>>,
    /// The app update download: only one runs, and `cancel_app_update` stops it.
    download: SingleRun,
}
