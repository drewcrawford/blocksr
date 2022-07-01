/*! Blocks that may be run more than once. */


use std::os::raw::{c_int, c_ulong};
use std::ffi::c_void;
use std::mem::MaybeUninit;

#[repr(C)]
#[derive(Debug)]
#[doc(hidden)]
pub struct BlockDescriptorMany {
    pub reserved: MaybeUninit<c_ulong>,
    pub size: c_ulong,
    /*
    For inline closures, we need to drop the closure ourselves.
    This requires the copy and dispose helpers.
     */
    pub copy_helper: extern "C" fn(dst: *mut blocksr::hidden::BlockLiteralManyEscape, src: *mut blocksr::hidden::BlockLiteralManyEscape),
    pub dispose_helper: extern "C" fn(src: *mut blocksr::hidden::BlockLiteralManyEscape),
}
#[doc(hidden)]
pub static mut BLOCK_DESCRIPTOR_MANY: BlockDescriptorMany = BlockDescriptorMany {
    reserved: MaybeUninit::uninit(),
    size: std::mem::size_of::<BlockLiteralManyEscape>() as u64,
    copy_helper: copy_helper,
    dispose_helper: dispose_helper,
};

extern "C" fn dispose_helper(src: *mut blocksr::hidden::BlockLiteralManyEscape) {
    println!("dispose_helper");
    unsafe{((*src).dispose)(src)}
}
extern "C" fn copy_helper(_dst: *mut blocksr::hidden::BlockLiteralManyEscape, _src: *mut blocksr::hidden::BlockLiteralManyEscape) {
    println!("copy_helper");
}

#[repr(C)]
#[derive(Debug)]
#[doc(hidden)]
pub struct Payload<C,E> {
    pub closure: C,
    pub environment: E,
}

#[repr(C)]
#[derive(Debug)]
#[doc(hidden)]
pub struct BlockLiteralManyEscape {
    pub isa: *const c_void,
    pub flags: c_int,
    pub reserved: c_int,
    //first arg to this fn ptr is &block_literal_1
    pub invoke: *const c_void,
    //pointer to static descriptor
    pub descriptor: *mut c_void,

    /*
    Because closures are anonymous types, it's tough to declare a static
    which depends on them (e.g. the block descriptor depends on their size).

    We could forego the static by allocating descriptors dynamically but then we run into the issue
    that each unique closure type may be of different size, etc.

    This is a boxed pointer to some Payload type.
    */
    pub payload: *mut c_void,

    pub dispose: fn(*mut BlockLiteralManyEscape),
}

/**
Declares a block that escapes and executes any number of times.  this is a typical pattern for IO.

```
    use blocksr::many_escaping_nonreentrant;
    many_escaping_nonreentrant!(MyBlock (environment: &mut (), arg: u8) -> u8);
    let f = unsafe{ MyBlock::new((),|_environment,_arg| {
        3
    })};
    //pass f somewhere...
```

`::new()` is declared unsafe.

# Safety

You must verify that
 * Arguments and return types are correct and in the expected order
     * Arguments and return types are FFI-safe (compiler usually warns)
 * Function will not be called in a re-entrant manner.  I believe this is required for FnMut, although I have not proven it.

The resulting block type is FFI-safe.  Typically, you pass a pointer to the block type (e.g., on the stack) into objc.
Typically, you want to declare the pointer type `Arguable` in objr to pass it into objc, e.g.

```ignore
many_escaping!(DataTaskCompletionHandler(data: *const NSData, response: *const NSURLResponse, error: *const NSError) -> ());
unsafe impl Arguable for &DataTaskCompletionHandler {}
```

# Environment

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

 */
#[macro_export]
macro_rules! many_escaping_nonreentrant(

    (
        $pub:vis $blockname: ident (environment: &mut $environment:ty $(,$a:ident : $A:ty)*) -> $R:ty
    ) => {


        //must be ffi-safe
        #[repr(transparent)]
        #[derive(Debug)]
        $pub struct $blockname(blocksr::hidden::BlockLiteralManyEscape);
        impl $blockname {

            ///Creates a new escaping block.
            ///
            /// # Safety
            /// You must verify that
            /// * Arguments and return types are correct and in the expected order
            ///     * Arguments and return types are FFI-safe (compiler usually warns)
            /// *  Function will not be called in a re-entrant manner.  I believe this is required for FnMut, although I have not proven it.
            /// The resulting block type is FFI-safe.  Typically, you pass a pointer to the block type (e.g., on the stack) into objc.
            pub unsafe fn new<C,E>(environment: E, f: C) -> Self where C: FnMut(&mut E, $($A),*) -> $R + Send + 'static {
                //This thunk is safe to call from C
                extern "C" fn invoke_thunk<G,H>(block: *mut blocksr::hidden::BlockLiteralManyEscape, $($a : $A),*) -> $R where G: FnMut(&mut H, $($A),*) -> $R + Send {
                    let payload_ptr = unsafe{(*block).payload} as *mut _ as *mut blocksr::hidden::Payload<G,H>;
                    let mut boxed_payload: Box<blocksr::hidden::Payload<G,H>> = unsafe {Box::from_raw(payload_ptr)};
                    let closure: &mut G = &mut boxed_payload.closure;
                    let environment: &mut H = &mut boxed_payload.environment;
                    let r = closure(environment, $($a),*);
                    std::mem::forget(boxed_payload);
                    r
                }

                fn dispose_thunk<G,H>(block: *mut blocksr::hidden::BlockLiteralManyEscape) {
                    let payload_ptr = unsafe{(*block).payload} as *mut _ as *mut blocksr::hidden::Payload<G,H>;
                    let mut boxed_payload: Box<blocksr::hidden::Payload<G,H>> = unsafe {Box::from_raw(payload_ptr)};
                    //drop
                }

                let thunk_fn: *const core::ffi::c_void = invoke_thunk::<C,E> as *const core::ffi::c_void;
                //make payload
                let payload = blocksr::hidden::Payload {
                    closure: f,
                    environment
                };
                //box payload
                let boxed_load = Box::new(payload);
                //note: this leak will be cleaned up by dispose
                let raw_load = Box::into_raw(boxed_load) as *mut _ as *mut core::ffi::c_void;
                let literal = blocksr::hidden::BlockLiteralManyEscape {
                    isa: &blocksr::hidden::_NSConcreteStackBlock,
                    flags: blocksr::hidden::BLOCK_HAS_STRET | blocksr::hidden::BLOCK_HAS_COPY_DISPOSE,
                    reserved: std::mem::MaybeUninit::uninit().assume_init(),
                    invoke: thunk_fn ,
                    descriptor: &mut blocksr::hidden::BLOCK_DESCRIPTOR_MANY as *mut _ as *mut core::ffi::c_void,
                    payload: raw_load,
                    dispose: dispose_thunk::<C,E>,
                };
                $blockname(literal)
            }

        }

    }
);