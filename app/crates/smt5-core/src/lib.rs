//! SiliconMetaTrader5 core: everything the `smt5` CLI and the desktop app share.
//!
//! * [`runtime`] — locating the `runtime/` folder and its `.env`
//! * [`exec`]    — running external commands (mockable for tests)
//! * [`provider`] — where the x86_64 container runs (Colima VM, Docker, SSH)
//! * [`docker`]  — docker / docker compose calls pinned to the provider's context
//! * [`api`]     — typed client for the in-container status API (:8002)
//! * [`doctor`]  — prerequisite and health checks (setup wizard step 1)
//! * [`native`]  — macOS native mode (Wine 11 + Rosetta 2, launchd agent)
//! * [`remote`]  — SSH to a server: targets, tunnels, host keys (Server mode)
//! * [`remote_setup`] — server preflight, packages, runtime upload, lifecycle scripts
//! * [`setup`]   — first-run `.env` (password, API token)

pub mod api;
pub mod docker;
pub mod doctor;
pub mod env;
pub mod exec;
pub mod macapp;
pub mod mode;
pub mod native;
pub mod provider;
pub mod remote;
pub mod remote_setup;
pub mod runtime;
pub mod setup;
pub mod util;

pub use api::ApiClient;
pub use docker::Docker;
pub use env::EnvFile;
pub use exec::{Cmd, Output, Runner, SystemRunner};
pub use provider::{HostInfo, Provider, ProviderKind};
pub use runtime::RuntimeDir;

/// Name of the runtime container (compose `container_name`).
pub const CONTAINER: &str = "siliconmt5";
