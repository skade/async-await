#![feature(rust_2018_preview, async_await, await_macro, futures_api, pin, arbitrary_self_types)]

use std::future::{Future, FutureObj};
use std::mem::PinMut;
use std::marker::Unpin;

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{sync_channel, SyncSender, SendError, Receiver};
use std::task::{
    self,
    Executor as ExecutorTrait,
    local_waker_from_nonlocal,
    Poll,
    SpawnErrorKind,
    SpawnObjError,
    Wake,
};

struct Executor {
    task_sender: SyncSender<Arc<Task>>,
    task_receiver: Receiver<Arc<Task>>,
}

impl<'a> ExecutorTrait for &'a Executor {
    fn spawn_obj(&mut self, future: FutureObj<'static, ()>)
        -> Result<(), SpawnObjError>
    {
        let task = Arc::new(Task {
            future: Mutex::new(Some(future)),
            task_sender: self.task_sender.clone(),
        });

        self.task_sender.send(task).map_err(|SendError(task)| {
            SpawnObjError {
                kind: SpawnErrorKind::shutdown(),
                future: task.future.lock().unwrap().take().unwrap(),
            }
        })
    }
}

struct Task {
    future: Mutex<Option<FutureObj<'static, ()>>>,
    task_sender: SyncSender<Arc<Task>>,
}

impl Wake for Task {
    fn wake(arc_self: &Arc<Self>) {
        let cloned = arc_self.clone();
        let _ = arc_self.task_sender.send(cloned);
    }
}

impl Executor {
    fn new() -> Self {
        let (task_sender, task_receiver) = sync_channel(1000);
        Executor { task_sender, task_receiver }
    }

    fn run(&self) {
        let mut executor = &*self;
        while let Ok(task) = self.task_receiver.recv() {
            let mut future_slot = task.future.lock().unwrap();
            if let Some(mut future) = future_slot.take() {
                // Should we use the ref version here? might be nice to start
                // w/o futures crate at first to show that it can be done,
                // and just mention that there's a simple function to avoid
                // the clone if anyone asks?
                let waker = local_waker_from_nonlocal(task.clone());
                let cx = &mut task::Context::new(&waker, &mut executor);
                if let Poll::Pending = PinMut::new(&mut future).poll(cx) {
                    *future_slot = Some(future);
                }
            }
        }
    }
}

struct WorkerThread<F> {
    function: Option<F>,
}

impl<F> WorkerThread<F>
where F: FnOnce() + Unpin + Send + Sync + 'static {
    fn new(function: F) -> Self {
        Self { function: Some(function) }
    }
}

impl<F> Future for WorkerThread<F>
where F: std::ops::FnOnce() + Unpin + Send + Sync + 'static {
    type Output = ();

    fn poll(mut self: std::mem::PinMut<Self>, ctx: &mut std::task::Context) -> std::task::Poll<()> {
        println!("future: poll called!");

        if self.function.is_some() {
            println!("future: spinning up thread!");

            let f = self.function.take().unwrap();

            let waker = ctx.waker().clone();

            std::thread::spawn(move || {
                println!("thread: started!");
                f();
                println!("thread: finished!");
                waker.wake();
            });

            println!("future: going to sleep!");

            std::task::Poll::Pending
        } else {
            println!("future: someone woke me up!");
            println!("future: give me coffeeeee!!!!");
            std::task::Poll::Ready(())
        }
    }
}


async fn thread() {
    let payload = || { println!("closure: I am the workload!") };
    
    let w = WorkerThread::new(payload);
    
    await!(w);

    println!("thread: after workerthread");
}

async fn hello() {
    println!("simple: I was executed asynchronously!");
}

fn main() {
    let executor = Executor::new();
    println!("main: Hello!");
    (&executor).spawn_obj(FutureObj::new(Box::new(
        thread()
    ))).unwrap();
    (&executor).spawn_obj(FutureObj::new(Box::new(
        hello()
    ))).unwrap();
    println!("main: Hello after you spawn!");

    executor.run();
}