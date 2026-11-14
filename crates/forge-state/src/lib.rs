//! forge-state: the GPUI-free view model of forge-app (owned by Forge Web
//! Developer). It reduces `forge_protocol` events into view state and turns
//! user intents into protocol commands. Everything here is testable on Linux;
//! forge-ui is a thin view over it. This crate must never depend on gpui.

mod app;
pub mod i18n;
pub mod ids;
pub mod layout;
mod onboarding;
pub mod outcome;
mod palette;
mod quit;
mod session;
mod settings;
mod sidebar;
mod sync;
mod timeline;

pub use app::*;
pub use layout::*;
pub use onboarding::*;
pub use outcome::*;
pub use palette::*;
pub use quit::*;
pub use session::*;
pub use settings::*;
pub use sidebar::*;
pub use timeline::*;
