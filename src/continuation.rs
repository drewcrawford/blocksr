/*!
Continuations help bind completion-based code into Rust async fns.  The intent
here is to work with block-based completion APIs, as that is a typical problem,
but the solution is by no means limited to that scope.

This module is spiritually similar to the idea in [SE-0300](https://github.com/apple/swift-evolution/blob/main/proposals/0300-continuation.md).
Basically, we have this [Continuation] type, which implements the language async stuff and you can return it or await on it.
That [Continuation] can then be 'completed' (what Swift calls 'resumed') explicitly.
This lets you capture the result from some completion block.

That highlevel picture is the same, although there are various details specific to an efficient Rust implementation.


*/
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::mem::MaybeUninit;
use std::future::Future;
use std::sync::{Mutex, MutexGuard};
use std::hint::unreachable_unchecked;
use std::marker::PhantomPinned;

///Structure in memory while a [Completion] is pending
struct SharedPending {
    ///Wake this type to stop pending
    waker: Waker
}
///The shared part of a [Completer], internal implementation type
///
/// This type is generally wrapped by a lock, so we expect 1 user at a time here
///
/// This is an internal state machine with various states mapping to possible situations
enum InternalCompleter<Result> {
    ///Ready to be polled
    Done(Result),
    ///Not ready to be polled
    Pending(SharedPending),
    ///internal implementation detail.  This should never
    ///escape an individual function call, and if it does we may UB
    Invalid,
    ///Already returned a result; we moved it out
    Gone
}
impl<Result> InternalCompleter<Result> {
    /// Marks the result as complete
    /// # Safety
    /// UB to call this more than once, or if the start situation is not Pending
    unsafe fn complete(&mut self, result: Result) {
        let mut local = InternalCompleter::Invalid;
        //-------------WARNING----------------------
        //needs to set self through all paths in fn!
        //--------------------------------------------
        std::mem::swap(&mut local, self);
        if let InternalCompleter::Pending(pending) = local {
            *self = InternalCompleter::Done(result);
            pending.waker.wake()
        }
        else {
            //UB to call this more than once
             unreachable_unchecked()
        }
        //don't swap back - self was set through the only reachable path in this fn
    }
    fn poll(&mut self, waker: &Waker) -> Poll<Result> {
        let mut local = InternalCompleter::Invalid;
        //-------------WARNING----------------------
        //needs to set self through all paths in fn!
        //--------------------------------------------
        std::mem::swap(self, &mut local);
        match local {
            InternalCompleter::Done(result) => {
                *self = InternalCompleter::Gone;
                Poll::Ready(result)
            }
            InternalCompleter::Pending(mut p) => {
                p.waker = waker.clone();
                *self = InternalCompleter::Pending(p);
                Poll::Pending
            }
            InternalCompleter::Gone => {
                panic!("Polled too many times!");
            }
            InternalCompleter::Invalid => {
                unsafe {
                    unreachable_unchecked()
                }
            }
        }
    }
}
///Owned completer type, this version is kept in our struct
struct OwnedCompleter<Result>(Mutex<InternalCompleter<Result>>,PhantomPinned);
impl<Result> OwnedCompleter<Result> {
    fn lock(&self) -> MutexGuard<'_, InternalCompleter<Result>> {
        self.0.lock().unwrap()
    }
}

pub struct Completer<Result: 'static>(Pin<&'static OwnedCompleter<Result>>);
impl<Result> Completer<Result> {
    ///Complete the continuation with the given result
    pub fn complete(self,result:Result) {
        let as_ref = self.0;
        //this can only be called once because it's a consuming fn
        unsafe{ as_ref.0.lock().unwrap().complete(result) };
    }
}

///Continuations are an implementation of [std::future::Future] that can be explicitly completed.
///
/// For more details, see [Continuation::new()]
///
/// Private type; this enum is wrapped for public visibility.
enum InternalContinuation<Begin,Begun,Result> {
    ///Task has not yet begun, Begin is a closure which begins it
    Begin(Begin),
    //Task has begun
    Begun(MaybeUninit<Begun>, Pin<Box<OwnedCompleter<Result>>>),
    /// Internal implementation detail, should never see this value
    Invalid
}
impl<Begin,Begun,Result> InternalContinuation<Begin,Begun,Result> where Begin: FnOnce(Completer<Result>) -> Begun, Result: 'static,Begun:'static,Begin:'static {
    ///Get a reference to the pinned completer
    ///
    /// # Safety
    /// 1.  This will UB if self is not Begun
    /// 2.  After calling this function, it is UB if the pin inside the Begun case is dropped, before `.complete()` is called on the resulting completer
    unsafe fn get_extended_completer(&self) -> Pin<&'static OwnedCompleter<Result>> {
        if let InternalContinuation::Begun(_, pin) = self {
            //This is a completely crazy lifetime extension on the pinned argument.  Effectively,
            //we upgrade it from its current lifetime, to the static lifetime, in a way that is totally unchecked.

            //This is a lot of nonsense normally, but it ought to be safe for a few reasons:
            //1.  We're extending from a pinned value, so in theory the life of the pointer is at least the life of the pin
            //2.  The life of the pin, while a bit tough to track, should be the life of the future.  It is created in Begun, and we stay in Begun forever
            //3.  This is largely going to be accessed by calling complete(), but
            //  3.1 You can only call it once
            //  3.2 By definition, the future has to stick around until it's called
            let t = Pin::new_unchecked(&*(pin.as_ref().get_ref() as *const _));
            t
        }
        else {
            unreachable_unchecked()
        }
    }
    ///# Safety
    ///
    /// UB if self is not Task::Begun
    unsafe fn set_begun(&mut self,begun: Begun) {
        let mut local = InternalContinuation::Invalid;
        //-------------WARNING----------------------
        //needs to set self through all paths in fn!
        //--------------------------------------------
        std::mem::swap(&mut local, self);
        if let InternalContinuation::Begun(_, completer) = local {
            *self = InternalContinuation::Begun(MaybeUninit::new(begun), completer)
        }
        else {
            unreachable_unchecked()
        }
    }

    fn poll_impl(&mut self, waker: &Waker)  -> Poll<Result>{
        let mut local = InternalContinuation::Invalid;
        //task is invalid here; must set back at end of func
        std::mem::swap(&mut local, self);
        let poll_result = match local {
            InternalContinuation::Begin(begin) => {
                eprintln!("p:0");
                let owned_completer = Box::pin(OwnedCompleter(
                    Mutex::new(InternalCompleter::Pending(SharedPending {
                        waker: waker.clone()
                    })),
                    PhantomPinned::default()
                ));

                local = InternalContinuation::Begun(MaybeUninit::uninit(), owned_completer);
                //get a completer ref to pass in
                //safe because local is Begun
                let completer = Completer(unsafe{ local.get_extended_completer()});
                //ok since local is Task::Begun here
                let begun = begin(completer);
                //ok since local is Task::Begun here
                unsafe {
                    local.set_begun(begun);
                }
                Poll::Pending
            }
            InternalContinuation::Begun(begun, completer) => {
                eprintln!("p:1");
                let result = {
                    let mut lock = completer.lock();
                    lock.poll(waker)
                };
                //move back
                local = InternalContinuation::Begun(begun, completer);
                result
            }
            InternalContinuation::Invalid => {
                eprintln!("p:2");
                unsafe {
                    unreachable_unchecked()
                }
            }
        };
        std::mem::swap(&mut local, self);
        eprintln!("p:3");

        poll_result
    }
}

impl<Begin,Begun,Result> Future for InternalContinuation<Begin,Begun,Result> where Begin: Unpin, Result: Unpin, Begun: Unpin, Begin: FnOnce(Completer<Result>) -> Begun, Result: 'static,Begun:'static,Begin:'static {
    type Output = Result;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let s = self.get_mut();
        s.poll_impl(cx.waker())
    }
}

///Continuations are an implementation of [std::future::Future] that can be explicitly completed.
///
/// For more details, see [Continuation::new()]
pub struct Continuation<Begin,Begun,Result>(InternalContinuation<Begin,Begun,Result>);
impl<Begin,Begun,Result> Future for Continuation<Begin,Begun,Result> where Begin: Unpin, Result: Unpin, Begun: Unpin, Begin: FnOnce(Completer<Result>) -> Begun,Result: 'static,Begun:'static,Begin:'static {
    type Output = Result;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().0.poll_impl(cx.waker())
    }

}
impl<Begin,Begun,Result> Continuation<Begin,Begun,Result> where Begin: Unpin, Result: Unpin, Begun: Unpin, Begin: FnOnce(Completer<Result>) -> Begun, Result: 'static {
    pub fn new(run: Begin) -> Self {
        Continuation(InternalContinuation::Begin(run))
    }
}

#[test] fn test_task() {
    let task = Continuation::new(|completer | {
        completer.complete(23)
    });
    let r = kiruna::test::test_await(task, std::time::Duration::from_secs(1));
    assert_eq!(r,23);
}