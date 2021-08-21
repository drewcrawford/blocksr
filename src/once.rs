use std::os::raw::{c_int,c_ulong};
use std::ffi::c_void;

#[repr(C)]
#[derive(Debug)]
#[doc(hidden)]
pub struct BlockDescriptorOnce {
    pub reserved: c_ulong,
    pub size: c_ulong,
    /*
     optional helper functions
        void (*copy_helper)(void *dst, void *src);     // IFF (1<<25)
        void (*dispose_helper)(void *src);             // IFF (1<<25)
        required ABI.2010.3.16
        const char *signature;                         // IFF (1<<30)
     */
}
#[repr(C)]
#[derive(Debug)]
#[doc(hidden)]
pub struct BlockLiteralOnce {
    pub isa: *const c_void,
    pub flags: c_int,
    pub reserved: c_int,
    //first arg to this fn ptr is &block_literal_1
    pub invoke: *const c_void,
    pub descriptor: *mut BlockDescriptorOnce,
    /*Capture list.  It's very tricky to do this in Rust.

    Because closures are anonymous types, it's tough to declare a static
    which depends on them (e.g. the block descriptor depends on their size).

    We could forego the static by allocating descriptors dynamically but if we're
    going to do that, we might as well box the tough part (the closure) instead.
    */
    pub closure: *const c_void,
}

pub static mut BLOCK_DESCRIPTOR_ONCE: blocksr::hidden::BlockDescriptorOnce = BlockDescriptorOnce {
    reserved: 0, //unsafe{std::mem::MaybeUninit::uninit().assume_init()} is unstable as const fn
    size: std::mem::size_of::<blocksr::hidden::BlockLiteralOnce>() as u64,
};

/**
Declares a block that escapes and executes once.  this is a typical pattern for completion handlers.

```
    use blocksr::once_escaping;
    once_escaping!(MyBlock (arg: u8) -> u8);
    let f = unsafe{ MyBlock::new(|_arg| {
        3
    })};
    //pass f somewhere...
```

`::new()` is declared unsafe.

# Safety

You must verify that
 * Arguments and return types are correct and in the expected order
     * Arguments and return types are FFI-safe (compiler usually warns)
 * Block will execute exactly once:
     * If ObjC executes the block several times, it's UB
     * If ObjC executes the block less than once, it is not UB, but it will leak.

The resulting block type is FFI-safe.  Typically, you pass a pointer to the block type (e.g., on the stack) into objc.
Typically, you want to declare the pointer type `Arguable` in objr to pass it into objc, e.g.

```ignore
once_escaping!(DataTaskCompletionHandler(data: *const NSData, response: *const NSURLResponse, error: *const NSError) -> ());
unsafe impl Arguable for &DataTaskCompletionHandler {}
```
*/
#[macro_export]
macro_rules! once_escaping(

    (
        $pub:vis $blockname: ident ($($a:ident : $A:ty),*) -> $R:ty
    ) => {
        //must be ffi-safe
        #[repr(transparent)]
        #[derive(Debug)]
        $pub struct $blockname(blocksr::hidden::BlockLiteralOnce);
        impl $blockname {
            ///Creates a new escaping block.
            ///
            /// # Safety
            /// You must verify that
            /// * Arguments and return types are correct and in the expected order
            ///     * Arguments and return types are FFI-safe (compiler usually warns)
            /// * Block will execute exactly once:
            ///     * If ObjC executes the block several times, it's UB
            ///     * If ObjC executes the block less than once, it is not UB, but it will leak.
            ///
            /// The resulting block type is FFI-safe.  Typically, you pass a pointer to the block type (e.g., on the stack) into objc.
            pub unsafe fn new<F>(f: F) -> Self where F: FnOnce($($A),*) -> $R + Send + 'static {
                //This thunk is safe to call from C
                extern "C" fn invoke_thunk<G>(block: *mut blocksr::hidden::BlockLiteralOnce, $($a : $A),*) -> $R where G: FnOnce($($A),*) -> $R + Send {
                    eprintln!("invoke_thunk");
                    let typed_ptr: *mut G = unsafe{ (*block).closure as *mut G};
                    let rust_fn = unsafe{ Box::from_raw(typed_ptr)};
                    rust_fn($($a),*)
                    //drop box
                }
                let boxed = Box::new(f);
                eprintln!("made boxed {:p}",boxed);
                let thunk_fn: *const core::ffi::c_void = invoke_thunk::<F> as *const core::ffi::c_void;
                let literal = blocksr::hidden::BlockLiteralOnce {
                    isa: &blocksr::hidden::_NSConcreteStackBlock,
                    flags: blocksr::hidden::BLOCK_HAS_STRET,
                    reserved: std::mem::MaybeUninit::uninit().assume_init(),
                    invoke: thunk_fn ,
                    descriptor: &mut blocksr::hidden::BLOCK_DESCRIPTOR_ONCE,
                    closure: Box::into_raw(boxed) as *mut core::ffi::c_void,
                };
                $blockname(literal)
            }

        }

    }
);





extern {
    #[doc(hidden)]
    pub static _NSConcreteStackBlock: c_void;
}

#[doc(hidden)]
pub const BLOCK_HAS_STRET: c_int = 1<<29;


#[test] fn make_escape() {
    once_escaping!(MyBlock (arg: u8) -> u8);
    let _f = unsafe{ MyBlock::new(|_arg| {
        3
    })};
}