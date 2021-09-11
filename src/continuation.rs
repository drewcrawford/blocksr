/*!
Continuations help bind completion-based code into Rust async fns.  The intent
here is to work with block-based completion APIs, as that is a typical problem,
but the solution is by no means limited to that scope.

This is similar to (and informed by) Apple's own Swift bridge for async methods, with broad compatability across
real-world Apple APIs. Compare to the idea in [SE-0300](https://github.com/apple/swift-evolution/blob/main/proposals/0300-continuation.md).
Basically, we have this [Continuation] type, which implements the language async stuff and you can return it or await on it.
That [Continuation] can then be 'completed' (what Swift calls 'resumed') explicitly.
This lets you capture the result from some completion block.

That highlevel picture is the same, although there are various details specific to an efficient Rust implementation.

This Rust version is self-contained, 200 lines, does not depend on Tokio and is tested against other async runtimes.

# Note to bindings authors

As a practical matter, you may wish to avoid declaring async fns directly, and either ship both `impl Future`  and completion-handler
versions as options, or only the completion-handler version and push the problem onto callers to use their own continuations.

The reason is that bindings generally involve an autoreleasepool as an argument

```compile_fail
async fn my_binding(pool: objr::&ActiveAutoreleasePool) -> u8;

//that is,
fn my_binding<'a>(pool: objr::&'a ActiveAutoreleasePool) -> impl Future<Output=u8> + 'a
```

This has a few weird implications:
1.  First, Rust checks you maintain the autoreleasepool across the suspension point, when that is not actually required
    and doing that may be challenging or create poor performance for some applications
2.  Second, since autorelease pools are thread-local, they are not [core::marker::Send] whereas
    usually we expect [std::future::Future]s to be sent to other threads.
3.  Third, since suspend/resume is essentially unordered, this may cause autoreleasepools not to be
    popped in pushed order, which is incorrect.

In my experience, there's usually not enough information to solve this at the bindings level, but it's easily
solved at some higher level.  For example, an application may have a strong intuition about whether to use a pool-per-completion
or not based on size of tasks, configuration of target queue, or other factors.

That said, I'm not the design police and there probably are good reasons to do it in some circumstances.
*/
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::future::Future;
use std::sync::{Mutex, Arc};
use std::hint::unreachable_unchecked;

///The shared part of a [Completer], internal implementation type
///
/// This type is generally wrapped by a lock, so we expect 1 user at a time in here.
///
/// This is an internal state machine with various states mapping to possible situations
enum InternalCompleter<Result> {
    ///Initial state
    NotPolled,
    ///Polled and pending, we can wake the waker to get updates.
    Pending(Waker),
    ///Future delivered a result which is
    Done(Result),
    ///internal implementation detail.  This should never
    ///escape an individual function call, and if it does we may UB
    Invalid,
    ///Already returned a result; we moved it out.
    Gone
}
impl<Result> InternalCompleter<Result> {
    /// Marks the result as complete
    /// # Safety
    /// UB to call this more than once, or if we're in state !(Polled || Pending)
    unsafe fn complete(&mut self, result: Result) {
        let mut local = InternalCompleter::Invalid;
        //-------------WARNING----------------------
        //needs to set self through all paths in fn!
        //--------------------------------------------
        std::mem::swap(&mut local, self);
        if let InternalCompleter::Pending(pending) = local {
            *self = InternalCompleter::Done(result);
            pending.wake()
        }
        else if let InternalCompleter::NotPolled = local {
            /* This case is somewhat counterintuitive.  In short, it's possible to complete a future before it's polled.

            This requires some explanation as it doesn't happen in normal Rust future design, where poll
            is the first opportunity to make progress.  I considered that design, but replaced it with this one
            after studying use sites.

            The issue with it is that the way your future will start, is by calling some objc method.  So it needs to have some
            `&Receiver`, `&Arguments`, etc.  Those would need to survive until future starts work, a.k.a. first poll.  Meaning we
            would have to either

            1.  Convert them to StrongCell and move them inside the future, which has various runtime overhead
            2.  Give the future a bunch of lifetime bounds for receiver/arguments.  Trouble here is that the future only needs
                to refer to the arguments until first poll, but the lifetime bounds on the container are forever.

            Instead of any of that, we simply allow the future to be completed first, and polled second.  With this design
            we can go ahead and make our `objc_binding(&Receiver,&Args, completer)` call inline in our objc method, as the first step.
            That call can in theory complete inline (e.g., not escaping), in which case the first poll will have data available.
             */
            *self = InternalCompleter::Done(result);
            //don't have to wake here
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
            InternalCompleter::Pending(_) | InternalCompleter::NotPolled => {
                //set new waker
                *self = InternalCompleter::Pending(waker.clone());
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

struct ThreadsafeCompleter<Result>(Mutex<InternalCompleter<Result>>);

///Completer is a type upon which you can call [Completer::complete] to provide the result of the continuation.
///
/// To get a copy of this type, call [Continuation::new].
//- note: This needs to be Arc because the future can be dropped before it completes, in which case
// we don't especially care about the result but we still want a consistent answer
pub struct Completer<Result>(Arc<ThreadsafeCompleter<Result>>);
impl<Result> Completer<Result> {
    ///Complete the continuation with the given result
    pub fn complete(self,result:Result) {
        unsafe {
            let reff = &*(self.0);
            //this can only be called once because it's a consuming fn
            reff.0.lock().unwrap().complete(result);
        }
    }
}
///Continuations are an implementation of [std::future::Future] that can be explicitly completed.
///
/// For more details, see [Continuation::new()]
pub struct Continuation<Accepted,Result> {
    completer: Completer<Result>,
    accepted: Option<Accepted>
}
impl<Accepted,Result> Future for Continuation<Accepted,Result>  {
    type Output = Result;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.completer.0.0.lock().unwrap().poll(cx.waker())
    }

}
impl<Accepted,Result> Continuation<Accepted,Result> {
    ///Create a new Continuation.
    ///
    /// This returns a tuple of (Continuation,Completer).  The Continuation can be awaited,
    /// the completer can be `.completed()`.  These operations may happen in any order.
    ///
    /// This type allows you to implement an async fn that wraps some block-based (or thread-based) API.  Here's
    /// a simple example:
    /// ```
    /// use blocksr::continuation::Continuation;
    /// async fn example() -> u8 {
    ///     //specifying types here lets us skip calling `accept`.  For more details, see docs
    ///     let (mut continuation,completer) = Continuation::<(),u8>::new();
    ///     //on another thread...
    ///     std::thread::spawn(||
    ///         //complete the continuation
    ///         completer.complete(23)
    ///     );
    ///     //back in the calling thread, await the continuation
    ///     continuation.await
    /// }
    /// ```
    pub fn new() -> (Self,Completer<Result>) {
       let continuation = Continuation {
            completer: Completer(Arc::new(ThreadsafeCompleter(
                Mutex::new(InternalCompleter::NotPolled),
            ))),
           accepted: None
        };
        let completer = Completer(continuation.completer.0.clone());
        (continuation,completer)
    }
    ///Causes the value specified to be moved inside the future.  The effect of this is that
    /// if the future is dropped, the value accepted will be dropped as well.  This lets you implement
    /// implicit cancelling by implementing Drop on some type and passing it in here.
    ///
    pub fn accept(&mut self, value: Accepted) {
        self.accepted = Some(value);
    }
}



#[test] fn test_task() {
    let (mut continuation,completer) = Continuation::new();
    continuation.accept(());
    completer.complete(23);
    let r = kiruna::test::test_await(continuation, std::time::Duration::from_secs(1));
    assert_eq!(r,23);
}