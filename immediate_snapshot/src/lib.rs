use std::collections::HashSet;
use std::ops::RangeBounds;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use process::{ProcessId, System, View};
use process_local::ProcessLocal;
use register::Register;

pub struct ImmediateSnapshot<T> {
    num_procs: usize,
    vals: Box<[Register<T>]>,
    regs: Box<[Register<usize>]>,

    // This uses Mutex but this is just to get the mutable reference to the inner vector; no lock
    // occurs.
    vals_local: ProcessLocal<Mutex<Box<[T]>>>,
    floor: ProcessLocal<AtomicUsize>,
}

impl<T: Clone> ImmediateSnapshot<T> {
    pub fn new(sys: &System, init: T) -> ImmediateSnapshot<T> {
        let n = sys.num_procs();
        let vals = (0..n)
            .map(|_| Register::new(sys, init.clone()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let regs = (0..n)
            .map(|_| Register::new(sys, n + 1))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let vals_local =
            ProcessLocal::new(sys, || Mutex::new(vec![init.clone(); n].into_boxed_slice()));
        let floor = ProcessLocal::new(sys, || AtomicUsize::new(n + 1));

        ImmediateSnapshot {
            num_procs: n,
            vals,
            regs,
            vals_local,
            floor,
        }
    }

    // TODO: this is one-shot implementation
    pub fn update_snapshot(&self, pid: ProcessId, value: T) -> View<T> {
        let n = self.num_procs;
        let i = pid.as_usize();
        self.vals[i].write(pid, value);
        let viewed = loop {
            // This is process local and no other thread writes to it; it is safe even if I
            // non-atomically increment the value.
            let mut floor = self.floor.get(pid).load(Ordering::SeqCst);
            floor -= 1;
            self.floor.get(pid).store(floor, Ordering::SeqCst);
            self.regs[i].write(pid, floor);

            let mut viewed = HashSet::new();

            for j in 0..n {
                let l = self.regs[j].read(pid);
                if l <= floor {
                    viewed.insert(j);
                }
            }

            if viewed.len() >= floor {
                break viewed;
            }
        };

        let mut val_local = self
            .vals_local
            .get(pid)
            .try_lock()
            .expect("internal error: this lock should never fail");
        for j in 0..n {
            if viewed.contains(&j) {
                val_local[j] = self.vals[j].read(pid);
            }
        }

        View::from_boxed_slice(val_local.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::ImmediateSnapshot;
    use process::System;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn basic_usage() {
        let sys = System::with_procs(16);
        let immed_snapshot = Arc::new(ImmediateSnapshot::new(&sys, -1));

        #[allow(clippy::needless_collect)]
        let hs: Vec<_> = sys
            .procs()
            .iter()
            .map(|p| {
                let immed_snapshot = Arc::clone(&immed_snapshot);
                p.run(move |pid| {
                    thread::sleep(Duration::from_millis(100));
                    immed_snapshot.update_snapshot(pid, pid.as_usize() as i32)
                })
            })
            .collect();
        let mut views: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();

        let num_procs = sys.num_procs();

        // When sorted, views should always be related by inclusion.
        views.sort_by_cached_key(|view| view.iter().filter(|&&v| v >= 0).count());
        for (a, b) in views.windows(2).map(|pair| (&*pair[0], &*pair[1])) {
            assert!((0..num_procs).all(|j| a[j] == -1 || a[j] == b[j]));
        }

        // When sorted, the last item should be the sequence of 0..num_procs
        assert_eq!(
            &**views.last().unwrap(),
            &*(0..num_procs as i32).collect::<Vec<_>>()
        );
    }
}
