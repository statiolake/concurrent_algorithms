use process::{ProcessId, System};

#[derive(Debug)]
pub struct ProcessLocal<T> {
    num_procs: usize,
    data: Box<[T]>,
}

impl<T> ProcessLocal<T> {
    pub fn get(&self, pid: ProcessId) -> &T {
        &self.data[pid.as_usize()]
    }

    pub fn as_slice(&self) -> &[T] {
        &*self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut *self.data
    }

    pub fn new<F>(sys: &System, factory: F) -> ProcessLocal<T>
    where
        F: Fn() -> T,
    {
        let num_procs = sys.num_procs();
        let data = (0..num_procs)
            .map(|_| factory())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ProcessLocal { num_procs, data }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use process::System;

    use crate::ProcessLocal;

    #[test]
    fn basic_usage() {
        const NUM_PROCESS: usize = 100;
        let sys = System::with_procs(NUM_PROCESS);
        let pls = Arc::new(ProcessLocal::new(&sys, || AtomicUsize::new(0)));

        #[allow(clippy::needless_collect)]
        let ths: Vec<_> = sys
            .procs()
            .iter()
            .map(|proc| {
                let pls = Arc::clone(&pls);
                proc.run(move |pid| {
                    pls.get(pid).store(pid.as_usize(), Ordering::SeqCst);
                })
            })
            .collect();
        ths.into_iter().for_each(|th| th.join().unwrap());

        let sum: usize = Arc::try_unwrap(pls)
            .unwrap()
            .as_mut_slice()
            .iter_mut()
            .map(|value| *value.get_mut())
            .sum();

        assert_eq!(sum, (0..NUM_PROCESS).sum());
    }
}
