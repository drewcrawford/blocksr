use std::os::raw::{c_int,c_ulong};
use std::ffi::c_void;
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;

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
pub struct BlockLiteralOnceEscape {
    pub isa: *const c_void,
    pub flags: c_int,
    pub reserved: MaybeUninit<c_int>,
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
    size: std::mem::size_of::<blocksr::hidden::BlockLiteralOnceEscape>() as u64,
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
        $pub struct $blockname(blocksr::hidden::BlockLiteralOnceEscape);
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
                extern "C" fn invoke_thunk<G>(block: *mut blocksr::hidden::BlockLiteralOnceEscape, $($a : $A),*) -> $R where G: FnOnce($($A),*) -> $R + Send {
                    let typed_ptr: *mut G = unsafe{ (*block).closure as *mut G};
                    let rust_fn = unsafe{ Box::from_raw(typed_ptr)};
                    rust_fn($($a),*)
                    //drop box
                }
                let boxed = Box::new(f);
                let thunk_fn: *const core::ffi::c_void = invoke_thunk::<F> as *const core::ffi::c_void;
                let literal = blocksr::hidden::BlockLiteralOnceEscape {
                    isa: &blocksr::hidden::_NSConcreteStackBlock,
                    flags: blocksr::hidden::BLOCK_HAS_STRET,
                    reserved: std::mem::MaybeUninit::uninit(),
                    invoke: thunk_fn ,
                    descriptor: &mut blocksr::hidden::BLOCK_DESCRIPTOR_ONCE,
                    closure: Box::into_raw(boxed) as *mut core::ffi::c_void,
                };
                $blockname(literal)
            }

        }

    }
);

#[repr(C)]
#[derive(Debug)]
#[doc(hidden)]
pub struct BlockLiteralNoEscape<C> {
    pub isa: *const c_void,
    pub flags: c_int,
    pub reserved: MaybeUninit<c_int>,
    //first arg to this fn ptr is &block_literal_1
    pub invoke: *const c_void,
    //in this situation, this points to the next field (struct is self-referential)
    pub descriptor: *mut BlockDescriptorOnce,
    //just put the descriptor on the stack!  mwahahaha
    pub inline_descriptor: BlockDescriptorOnce,
    //closure stored inline for this situation
    pub closure_inline: C,
    pub pinned: PhantomPinned,
}

/**
Declares a block that doesn't escape and executes once.  this is a typical pattern for `dispatch_sync`.

In this case we try to store the block on the stack.  To accomplish this, the block must be pinned.

Here's a complete example:

```
    use core::pin::Pin;
    use core::mem::MaybeUninit;
    use blocksr::once_noescape;
    //declare our block type
    once_noescape!(MyBlock(arg: u8) -> u8);

    //put block value on the stack
    let mut block_value = MaybeUninit::uninit();
    //pin to the stack.  By using the same variable name here, we guarantee that the original value cannot be moved
    //because there's no longer any way to access it
    let block_value = unsafe{ Pin::new_unchecked(&mut block_value) };

    //Initialize the block.  The argument here is uninitialized memory, and we return an initialized pointer to the same memory.
    let _f = unsafe { MyBlock::new(block_value, |_arg| {
        3
    }) };
    //pass _f somewhere...
```

`::new()` is declared unsafe.

# Safety

You must verify that
 * Arguments and return types are correct and in the expected order
     * Arguments and return types are FFI-safe (compiler usually warns)
 * Block will execute at most once:
     * If ObjC executes the block several times, it's UB

The resulting block type is FFI-safe.  Typically, you pass a pointer to the block type (e.g., on the stack) into objc.
Typically, you want to declare the pointer type `Arguable` in objr to pass it into objc, e.g.

```ignore
once_noescape!(MyBlock(data: *const NSData) -> ());
unsafe impl Arguable for &DataTaskCompletionHandler {}
```
 */
#[macro_export]
macro_rules! once_noescape(

    (
        $pub:vis $blockname: ident ($($a:ident : $A:ty),*) -> $R:ty
    ) => {
        //must be ffi-safe
        #[repr(transparent)]
        #[derive(Debug)]
        #[allow(non_snake_case)] //ex nw_parameters_configure_protocol_block_t
        $pub struct $blockname<F>(blocksr::hidden::BlockLiteralNoEscape<F>);
        impl<F> $blockname<F> {
            ///Creates a new escaping block.
            ///
            /// # Safety
            /// You must verify that
            //  * Arguments and return types are correct and in the expected order
            //      * Arguments and return types are FFI-safe (compiler usually warns)
            //  * Block will execute at most once:
            //      * If ObjC executes the block several times, it's UB
            ///
            /// The resulting block type is FFI-safe.  Typically, you pass a pointer to the block type (e.g., on the stack) into objc.
            pub unsafe fn new<'a>(into: core::pin::Pin<&'a mut core::mem::MaybeUninit<Self>>, f: F) -> core::pin::Pin<&'a Self> where F: FnOnce($($A),*) -> $R + Send {
                use blocksr::hidden::BlockLiteralNoEscape;
                use core::mem::MaybeUninit;
                use core::pin::Pin;
                //This thunk is safe to call from C
                extern "C" fn invoke_thunk<G>(block: *mut BlockLiteralNoEscape<G>, $($a : $A),*) -> $R where G: FnOnce($($A),*) -> $R + Send {
                    /*
                    This should be safe because:
                    * block is valid for reads
                    * block ought to be properly aligned, initialized, etc.
                    * nobody else is going to read block again; in particular we know that the thunk will be called once,
                    there is no dispose handler, etc
                     */
                    let read_owned = unsafe{std::ptr::read(block)};
                    (read_owned.closure_inline)($($a),*)
                    //drop read_owned
                }
                let thunk_fn: *const core::ffi::c_void = invoke_thunk::<F> as *const core::ffi::c_void;
                let mut literal = BlockLiteralNoEscape {
                    isa: &blocksr::hidden::_NSConcreteStackBlock,
                    flags: blocksr::hidden::BLOCK_HAS_STRET,
                    reserved: std::mem::MaybeUninit::uninit(),
                    invoke: thunk_fn ,
                    descriptor: std::ptr::null_mut(),
                    inline_descriptor: blocksr::hidden::BlockDescriptorOnce {
                        reserved: 0, //seems defined as NULL
                        size: std::mem::size_of::<BlockLiteralNoEscape<F>>() as u64
                    },
                    closure_inline: f,
                    pinned: std::marker::PhantomPinned,
                };
                //fixup self-referential pointer
                literal.descriptor = &mut literal.inline_descriptor;
                //should be ok because we are initializing the object
                let magic_ptr = into.get_unchecked_mut();
                *magic_ptr  = MaybeUninit::new($blockname(literal));
                //tell rust we're not worried about returning a temporary
                let raw_ptr: *const Self = magic_ptr.assume_init_ref();
                Pin::new_unchecked(&*raw_ptr)
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
#[doc(hidden)]
pub const BLOCK_HAS_COPY_DISPOSE: c_int = 1 << 25;
#[doc(hidden)]
pub const BLOCK_IS_NOESCAPE: c_int = 1<<23;

#[doc(hidden)]
pub const BLOCK_IS_GLOBAL: c_int = 1<<28;


#[test] fn make_escape() {
    once_escaping!(MyBlock (arg: u8) -> u8);
    let _f = unsafe{ MyBlock::new(|_arg| {
        3
    })};
}

#[test] fn make_noescape() {
    use core::pin::Pin;
    use std::mem::MaybeUninit;
    let mut block_value = MaybeUninit::uninit();
    let block_value = unsafe{ Pin::new_unchecked(&mut block_value) };

    once_noescape!(MyBlock(arg: u8) -> u8);
    let _f = unsafe { MyBlock::new(block_value, |_arg| {
        3
    })

    };
}