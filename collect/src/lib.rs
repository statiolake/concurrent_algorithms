use process::{ProcessId, System, View};
use register::Register;

pub struct Collect<T> {
    regs: Box<[Register<T>]>,
}

impl<T: Clone> Collect<T> {
    pub fn new(sys: &System, init: T) -> Collect<T> {
        let regs = (0..sys.num_procs())
            .map(|_| Register::new(sys, init.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Collect { regs }
    }

    pub fn collect(&self, pid: ProcessId) -> View<T> {
        let values: Vec<_> = (0..self.regs.len())
            .map(|id| self.regs[id].read(pid))
            .collect();
        View::from_boxed_slice(values.into_boxed_slice())
    }
}

impl<T> Collect<T> {
    pub fn store(&self, pid: ProcessId, new_value: T) {
        // store value at position id
        self.regs[pid.as_usize()].write(pid, new_value);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use process::System;

    use crate::Collect;

    #[test]
    fn simple_collect() {
        let sys = System::with_procs(128);
        let collect = Arc::new(Collect::new(&sys, 0));

        // False-positive clippy lint for thread handle
        // see: https://github.com/rust-lang/rust-clippy/issues/7207
        #[allow(clippy::needless_collect)]
        let hs: Vec<_> = sys
            .procs()
            .iter()
            .map(|proc| {
                let collect = Arc::clone(&collect);
                proc.run(move |pid| {
                    collect.store(pid, pid.as_usize());
                })
            })
            .collect();
        hs.into_iter().for_each(|h| h.join().unwrap());

        let num_procs = sys.num_procs();
        sys.procs()[0]
            .run(move |pid| {
                assert_eq!(
                    collect.collect(pid).into_inner(),
                    (0..num_procs).collect::<Vec<_>>().into_boxed_slice()
                );
            })
            .join()
            .unwrap();
    }
}
