# Drew's Rust library for (clang/objc) blocks.
![logo](art/logo.png)


This is my Rust crate for using [blocks](https://en.wikipedia.org/wiki/Blocks_(C_language_extension)), the Apple C extension
often used with ObjC.  This crate may be compared with the more popular [block](https://crates.io/crates/block) crate.

This crate is part of the [objr expanded universe universe](https://github.com/drewcrawford/objr#objr-expanded-universe) which provide low-level, zero-cost abstractions
for Apple platform features that mimic code from first-party compilers.  Distinctive features of this library include:

* Every block is a distinct newtype, creating a richer typesystem that unlocks new compile-time optimizations
    * In Rust, blocks may be `FnOnce`, `FnMut` (implemented), or `Fn` (planned), unlocking the full Rust typesystem
    * In C/ObjC, blocks may escape or not, unlocking various optimizations used by real C/ObjC compilers
    * C/ObjC is a giant ball of unsafe code, and most direct use of this crate is also unsafe.  Bindings authors are encouraged to wrap
      safe API based on their local knowledge.
    * Ergonomic macros for quickly binding new platform APIs
* Permissively licensed

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

## Many escaping, environment

In ObjC, blocks have a lifetime that extends beyond any single invocation, and are dropped after the block is dropped.
In Rust, types cannot generally be moved into an `Fn` or `FnMut` closure, as the syntax moves them into an *invocation*,
of which there are an arbitrary number.

Because this mismatch in usecase is quite common, the `many` macros have an additional component, the *environment*.  
The environment is moved into our block *as a whole* upon creation, rather than any single invocation, and a reference is passed
as the first argument to the closure.

In an `FnMut` context we can mutate the environment:

```rust
use blocksr::many_escaping_nonreentrant;
many_escaping_nonreentrant!(MyBlock (environment: &mut u8) -> ());
let f = unsafe{ MyBlock::new(23, |environment| {
    *environment += 1
})};
//pass f somewhere...
```

The environment is dropped when the block is dropped, with assistance from the ObjC runtime.  This will occur
sometime after the last execution.
