extern crate self as blocksr;
mod once;
#[cfg(feature="continuation")]
pub mod continuation;

#[doc(hidden)]
pub mod hidden {
    pub use super::once::{BlockLiteralOnce,BlockDescriptorOnce,_NSConcreteStackBlock,BLOCK_DESCRIPTOR_ONCE,BLOCK_HAS_STRET};
}


