extern crate num_cpus;

use std::sync::{Arc, Mutex, Condvar};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::collections::VecDeque;
use std::thread;
use std::time::Duration;

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

type Truck<'a> = Box<FnBox + Send + 'a>;

#[derive(Clone)]
pub struct Pool {
    inner: Arc<Inner>,
}

struct Inner {
    queue: Mutex<VecDeque<Truck<'static>>>,
    condvar: Condvar,
    active: AtomicUsize,
    waiting: AtomicUsize,
    min_num: usize,
}

struct Count<'a> {
    num: &'a AtomicUsize,
}

impl<'a> Count<'a> {
    fn add(num: &'a AtomicUsize) -> Count<'a> {
        num.fetch_add(1, Ordering::Release);
        
        Count {
            num: num,
        }
    }
}

impl<'a> Drop for Count<'a> {
    fn drop(&mut self) {
        self.num.fetch_sub(1, Ordering::Release);
    }
}

impl Pool {
    pub fn new() -> Pool {
        let cpu_num = num_cpus::get();

        let pool = Pool {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                condvar: Condvar::new(),
                active: AtomicUsize::new(0),
                waiting: AtomicUsize::new(0),
                min_num: cpu_num,
            }),
        };

        for _ in 0..cpu_num {
            pool.thread(None);
        }

        pool
    }

    pub fn with_capacity(n: usize) -> Pool {
        let pool = Pool {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                condvar: Condvar::new(),
                active: AtomicUsize::new(0),
                waiting: AtomicUsize::new(0),
                min_num: n,
            })
        };

        for _ in 0..n {
            pool.thread(None);
        }

        pool
    }
   
    pub fn spawn<F>(&self, handle: F)
        where F: FnOnce() + Send + 'static
    {
        let mut queue = self.inner.queue.lock().unwrap();

        if self.inner.waiting.load(Ordering::Acquire) == 0 {
            self.thread(Some(Box::new(handle)));
        } else {
            queue.push_back(Box::new(handle));
            self.inner.condvar.notify_one();
        }
    }

    fn thread(&self, handle: Option<Truck<'static>>) {
        let inner = self.inner.clone();

        thread::spawn(move || {
            let inner = inner;
            let _active = Count::add(&inner.active);

            if let Some(h) = handle {
                h.call_box();
            }

            loop {
                let handle = {
                    let mut queue = inner.queue.lock().unwrap();

                    let handle;

                    loop {
                        if let Some(front) = queue.pop_front() {
                            handle = front;
                            break;
                        }

                        let _waiting = Count::add(&inner.waiting);

                        if inner.active.load(Ordering::Acquire) <= inner.min_num {
                            queue = inner.condvar.wait(queue).unwrap();
                        } else {
                            let (q, wait) = inner.condvar.wait_timeout(queue, Duration::from_secs(30)).unwrap();
                            queue = q;

                            if wait.timed_out() && queue.is_empty() && inner.active.load(Ordering::Acquire) > inner.min_num {
                                return;
                            }
                        }
                    }

                    handle
                };

                handle.call_box();
            }
        });
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.inner.active.store(usize::max_value(), Ordering::Release);
        self.inner.condvar.notify_all();
    }
}

#[test]
fn test() {
    let thread_pool = Pool::new();

    for _ in 0..100 {
    
        let mut a: Vec<i32> = Vec::new();

        thread_pool.spawn(move || {
            a.push(123);
        });

    }

    thread::sleep(Duration::from_secs(2));
}
