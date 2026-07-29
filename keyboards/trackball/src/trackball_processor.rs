#[cfg(trackball_mini_v30)]
#[path = "trackball_processor/mini_v30.rs"]
mod profile;

#[cfg(trackball_mini_v31)]
#[path = "trackball_processor/mini_v31.rs"]
mod profile;

#[cfg(trackball_royale)]
#[path = "trackball_processor/royale.rs"]
mod profile;

pub use profile::TrackballModeProcessor;
