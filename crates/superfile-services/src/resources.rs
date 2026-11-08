//! Process-wide policy for buffers owned by Superfile. Framework allocations are separate.
use std::sync::OnceLock;
pub use superfile_core::resources::*;

pub fn pool() -> &'static ResourcePool {
    static POOL: OnceLock<ResourcePool> = OnceLock::new();
    POOL.get_or_init(|| {
        ResourcePool::with_budgets(
            crate::limits::RESOURCES_CACHE_BYTES,
            crate::limits::RESOURCES_WORKING_BYTES,
        )
    })
}
