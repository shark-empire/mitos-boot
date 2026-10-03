//! The renderer answers: "What do we draw?" (§7)

pub mod compositor;
pub mod effects;
pub mod renderer;
pub mod surface;
pub mod text;
pub mod texture;

pub use renderer::{Filter, Renderer};
pub use surface::{PixelBuffer, Rect};
pub use texture::Texture;