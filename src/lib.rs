// SPDX-License-Identifier: MIT OR Apache-2.0
/*!
# Drew's Rust library for (clang/objc) blocks.

This is my Rust crate for using [blocks](https://en.wikipedia.org/wiki/Blocks_(C_language_extension)), the Apple C extension
often used with ObjC.  This crate may be compared with the more popular [block](https://crates.io/crates/block) crate.

This crate is part of the [objr expanded universe universe](https://github.com/drewcrawford/objr#objr-expanded-universe) which provide low-level, zero-cost abstractions
for Apple platform features that mimic code from first-party compilers.  Distinctive features of this library include:

* Every block is a distinct newtype, creating a richer typesystem that unlocks new compile-time optimizations
   * In Rust, blocks may be [FnOnce] (implemented), [Fn], or [FnMut] (planned), unlocking the full Rust typesystem
   * In C/ObjC, blocks may escape (implemented) or not escape (planned), unlocking various optimizations used by real C/ObjC compilers
   * C/ObjC is a giant ball of unsafe code, and most direct use of this crate is also unsafe.  Bindings authors are encouraged to wrap
  safe API based on their local knowledge.
   * Ergonomic macros for quickly binding new platform APIs
 * The `continuation` feature (off by default) bridges block-based completion handlers to Rust `async fn`s.
     * This is similar to (and informed by) Apple's own Swift bridge for async methods, with broad compatability across
       real-world Apple APIs.
     * This Rust version is self-contained, 200 lines, does not depend on Tokio and is tested against other async runtimes.
* Free for noncommercial or "small commercial" use

# Examples

## Escaping block

```
use blocksr::once_escaping;
once_escaping!(MyBlock (arg: u8) -> u8);
let f = unsafe{ MyBlock::new(|_arg| {
    3
})};
//pass f somewhere...
```


*/
extern crate self as blocksr;
extern crate core;

mod once;

mod many;

#[doc(hidden)]
pub mod hidden {
    pub use super::once::{BlockLiteralOnceEscape, BlockDescriptorOnce, _NSConcreteStackBlock, BLOCK_DESCRIPTOR_ONCE, BLOCK_HAS_STRET, BLOCK_HAS_COPY_DISPOSE, BLOCK_IS_GLOBAL, BLOCK_IS_NOESCAPE, BlockLiteralNoEscape};
    pub use super::many::{BlockDescriptorMany,BlockLiteralManyEscape,Payload,BLOCK_DESCRIPTOR_MANY};
}


