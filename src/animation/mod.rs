//! Transitions (§16). The video decides what the owl *does*; this subsystem
//! decides how things *enter and leave the screen*.

pub mod easing;
pub mod fade;
pub mod timeline;
pub mod transition;

pub use timeline::{Phase, SplashTimeline};