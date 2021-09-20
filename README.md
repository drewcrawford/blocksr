# Drew's Rust library for (clang/objc) blocks.

This is my Rust crate for using [blocks](https://en.wikipedia.org/wiki/Blocks_(C_language_extension)), the Apple C extension
often used with ObjC.  This crate may be compared with the more popular [block](https://crates.io/crates/block) crate.

This crate is part of the [objr expanded universe universe](https://github.com/drewcrawford/objr#objr-expanded-universe) which provide low-level, zero-cost abstractions
for Apple platform features that mimic code from first-party compilers.  Distinctive features of this library include:

* Every block is a distinct newtype, creating a richer typesystem that unlocks new compile-time optimizations
    * In Rust, blocks may be `FnOnce` (implemented), `Fn`, or `FnMut` (planned), unlocking the full Rust typesystem
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

```rust
use blocksr::once_escaping;
once_escaping!(MyBlock (arg: u8) -> u8);
let f = unsafe{ MyBlock::new(|_arg| {
    3
})};
//pass f somewhere...
```

## Continuations

```rust
//only available when enabled
#[cfg(feature="continuation")]
mod f {
    use blocksr::continuation::Continuation;
    async fn example() -> u8 {
        //specifying types here lets us skip calling `accept()`.  For more details, see docs
        let (mut continuation,completer) = Continuation::<(),u8>::new();
        //on another thread...
        std::thread::spawn(||
            //complete the continuation
            completer.complete(23)
        );
        //back in the calling thread, await the continuation
        continuation.await
    }
}
```