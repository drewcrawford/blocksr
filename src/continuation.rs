/*!
Completions help interface blocks with Rust async.

The idea here is very similar to the one in the [SE-0300](https://github.com/apple/swift-evolution/blob/main/proposals/0300-continuation.md) proposal,
effectively we vend a type that can be used from a block to 
*/
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::mem::MaybeUninit;
use std::future::Future;
use std::sync::{Mutex, MutexGuard};
use std::hint::unreachable_unchecked;

///Structure in memory while a [Completion] is pending
struct Pending<Begun> {
    ///Return type of Begin, this lets cancel an operation by dropping some type
    begun: MaybeUninit<Begun>,
    ///Wake this type to stop pending
    waker: Waker
}
///The shared part of a completion,
enum InternalCompleter<Begun,Result> {
    Done(Result),
    Pending(Pending<Begun>),
    ///internal implementation detail
    Invalid,
    ///Already returned a result
    Gone
}
impl<Begun,Result> InternalCompleter<Begun,Result> {
    /// # Safety
    /// UB to call this more than once
    unsafe fn complete(&mut self, result: Result) {
        let mut local = InternalCompleter::Invalid;
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
        //needs to set self through all paths in fn!
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
    ///Sets the begun type
    fn set_begun(&mut self, begun: Begun) {
        //in theory, Done or Pending are possiblities here, as the completer was already passed into the closure.
        match self {
            InternalCompleter::Done(_) | InternalCompleter::Gone => {
                //if the closure returned inline, this set should be a no-op?
            }
            InternalCompleter::Pending(pending) => {
                pending.begun = MaybeUninit::new(begun);
            }
            InternalCompleter::Invalid => {
                //this case should never escape
                unsafe {
                    unreachable_unchecked()
                }
            }
        }
    }
}
///Threadsafe wrapper
struct SharedCompleter<Begun,Result>(Mutex<InternalCompleter<Begun,Result>>);
impl<Begun,Result> SharedCompleter<Begun,Result> {
    ///# Safety
    /// It is UB to call this more than once.
    unsafe fn complete(&self,result:Result) {
        let mut lock = self.0.lock().unwrap();
        //UB to call this more than once
        lock.complete(result);
    }

    fn lock(&self) -> MutexGuard<'_, InternalCompleter<Begun,Result>> {
        self.0.lock().unwrap()
    }
}

enum Task<Begin,Begun,Result> {
    ///Task has not yet begun, Begin is a closure which begins it
    Begin(Begin),

    Begun(SharedCompleter<Begun,Result>),
    /// Internal implementation detail, should never see this value
    Invalid
}
impl<Begin,Begun,Result> Task<Begin,Begun,Result> where Begin: FnOnce(&SharedCompleter<Begun,Result>) -> Begun {
    ///# Safety
    ///
    /// Can only call on Task::Begun
    unsafe fn set_begun(&mut self,begun: Begun) {
        if let Task::Begun(completer) = self {
            completer.lock().set_begun(begun);
        }
        else {
            unreachable_unchecked()
        }
    }
    ///# Safety
    ///
    /// Can only call on Task::Begun
    unsafe fn completer(&self) -> &SharedCompleter<Begun,Result> {
        if let Task::Begun(completer) = self {
            completer
        }
        else {
            unreachable_unchecked()
        }
    }
    fn poll_impl(&mut self, waker: &Waker)  -> Poll<Result>  where Begin: FnOnce(&SharedCompleter<Begun,Result>) -> Begun {
        let mut local = Task::Invalid;
        //task is invalid here; must set back at end of func
        std::mem::swap(&mut local, self);
        let poll_result = match local {
            Task::Begin(begin) => {
                let completer = SharedCompleter(
                    Mutex::new(InternalCompleter::Pending(Pending {
                        begun: MaybeUninit::uninit(),
                        waker: waker.clone()
                    }))
                );
                local = Task::Begun(completer);

                //ok since local is Task::Begun here
                let begun = begin(unsafe{ local.completer() });
                //ok since local is Task::Begun here
                unsafe {
                    local.set_begun(begun);
                }
                Poll::Pending
            }
            Task::Begun(begun) => {
                let result = {
                    let mut lock = begun.lock();
                    lock.poll(waker)
                };
                //move back
                local = Task::Begun(begun);
                result
            }
            Task::Invalid => {
                unsafe {
                    unreachable_unchecked()
                }
            }
        };
        std::mem::swap(&mut local, self);
        poll_result
    }
    pub fn new(run: Begin) -> Self {
        Task::Begin(run)
    }
}

impl<Begin,Begun,Result> Future for Task<Begin,Begun,Result> where Begin: Unpin, Result: Unpin,Begun: Unpin, Begin: FnOnce(&SharedCompleter<Begun,Result>) -> Begun {
    type Output = Result;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let s = self.get_mut();
        s.poll_impl(cx.waker())
    }
}

#[test] fn test_task() {
    let task = Task::new(|completer | {
        unsafe{ completer.complete(23) }
    });
    let r = kiruna::test::test_await(task, std::time::Duration::from_secs(1));
    assert_eq!(r,23);
}