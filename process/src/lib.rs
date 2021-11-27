use std::marker::PhantomData;
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread::{self, JoinHandle};

#[derive(Debug)]
pub struct System {
    procs: Vec<Process>,
}

impl System {
    pub fn with_procs(n: usize) -> System {
        let procs = (0..n).map(Process::new).collect();
        System { procs }
    }

    pub fn num_procs(&self) -> usize {
        self.procs.len()
    }

    pub fn procs(&self) -> &[Process] {
        &self.procs
    }
}

#[derive(Debug)]
pub struct Process {
    id: usize,
    running: Arc<Mutex<()>>,
}

impl Process {
    fn new(id: usize) -> Process {
        let running = Arc::new(Mutex::new(()));
        Process { id, running }
    }

    pub fn run<F, T>(&self, f: F) -> JoinHandle<T>
    where
        F: FnOnce(ProcessId) -> T + Send + 'static,
        T: Send + 'static,
    {
        let id = self.id;
        let running = Arc::clone(&self.running);
        thread::spawn(move || {
            // SAFETY: ProcessId is created by the ID associated to the process itself.
            let id = unsafe { ProcessId::new(id) };
            let _guard = match running.try_lock() {
                Ok(guard) => guard,
                Err(err) => match err {
                    TryLockError::Poisoned(_) => panic!("This process was not working"),
                    TryLockError::WouldBlock => panic!("This process is already running"),
                },
            };

            f(id)
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessId {
    id: usize,

    // Marker to make ProcessId !Send + !Sync so that it can't be passed outside of the closure.
    _marker: PhantomData<*const ()>,
}

impl ProcessId {
    /// # Safety
    ///
    /// It is up to the caller to guarantee the ID is in the valid range (0..num_procs) and the
    /// value is the same with the ID of a process on which the code is currently running.
    unsafe fn new(id: usize) -> ProcessId {
        ProcessId {
            id,
            _marker: PhantomData,
        }
    }

    pub fn as_usize(&self) -> usize {
        self.id
    }
}

#[derive(Debug)]
pub struct View<T> {
    values: Box<[T]>,
}

impl<T> View<T> {
    pub fn from_boxed_slice(values: Box<[T]>) -> View<T> {
        View { values }
    }

    pub fn as_slice(&self) -> &[T] {
        &*self.values
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut *self.values
    }

    pub fn get(&self, pid: ProcessId) -> &T {
        &self.values[pid.as_usize()]
    }

    pub fn get_mut(&mut self, pid: ProcessId) -> &mut T {
        &mut self.values[pid.as_usize()]
    }

    pub fn into_inner(self) -> Box<[T]> {
        self.values
    }
}

impl<T> Index<ProcessId> for View<T> {
    type Output = T;

    fn index(&self, index: ProcessId) -> &T {
        self.get(index)
    }
}

impl<T> IndexMut<ProcessId> for View<T> {
    fn index_mut(&mut self, index: ProcessId) -> &mut T {
        self.get_mut(index)
    }
}

impl<T> Deref for View<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &*self.values
    }
}

impl<T> DerefMut for View<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut *self.values
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use crate::System;

    #[test]
    fn basic_usage() {
        let sys = System::with_procs(16);

        #[allow(clippy::needless_collect)]
        let hs: Vec<_> = sys
            .procs()
            .iter()
            .map(|proc| {
                proc.run(move |_pid| {
                    // some process specific operations
                })
            })
            .collect();
        hs.into_iter().for_each(|h| h.join().unwrap());
    }

    #[test]
    #[should_panic]
    fn forbid_multi_run() {
        let sys = System::with_procs(1);
        let proc = &sys.procs()[0];

        // when forget to join ...
        proc.run(|_| {
            thread::sleep(Duration::from_secs(100));
        });

        // (wait to ensure the above thread is starting earlier)
        thread::sleep(Duration::from_millis(100));

        // ... it should panic here
        proc.run(|_| {}).join().unwrap();
    }
}
