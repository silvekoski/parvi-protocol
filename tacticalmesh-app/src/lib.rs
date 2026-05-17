pub mod messages;
pub mod state;
pub mod crdt;
pub mod image_cache;
pub mod image_codec;
pub mod mock;
pub mod demo;
pub mod tui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Bulk = 0,
    Normal = 1,
    Critical = 2,
}
