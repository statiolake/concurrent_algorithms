use std::sync::atomic::{AtomicUsize, Ordering};

use process::{ProcessId, System, View};
use process_local::ProcessLocal;
use register::Register;

pub struct Snapshot<T> {
    sn: ProcessLocal<AtomicUsize>,
    regs: Box<[Register<(T, usize)>]>,
}

impl<T: Clone> Snapshot<T> {
    pub fn new(sys: &System, init: T) -> Snapshot<T> {
        let sn = ProcessLocal::new(sys, || AtomicUsize::new(0));
        let regs = (0..sys.num_procs())
            .map(|_| Register::new(sys, (init.clone(), 0)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Snapshot { sn, regs }
    }

    pub fn update(&self, pid: ProcessId, value: T) {
        // self.sn is never written from another thread; so I don't have to use fetch_add() for
        // this.
        let mut sn = self.sn.get(pid).load(Ordering::SeqCst);
        sn += 1;
        self.sn.get(pid).store(sn, Ordering::SeqCst);
        self.regs[pid.as_usize()].write(pid, (value, sn));
    }

    pub fn snapshot(&self, pid: ProcessId) -> View<T> {
        let mut aa = self.scan(pid);
        loop {
            let bb = self.scan(pid);

            // double scan
            if unchanged(&aa, &bb) {
                return View::from_boxed_slice(
                    aa.into_iter()
                        .map(|(v, _)| v)
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                );
            }

            aa = bb;
        }
    }

    fn scan(&self, pid: ProcessId) -> Vec<(T, usize)> {
        self.regs
            .iter()
            .map(|reg| reg.read(pid))
            .collect::<Vec<_>>()
    }
}

fn unchanged<T>(aa: &[(T, usize)], bb: &[(T, usize)]) -> bool {
    let aa = aa.iter().map(|(_, sn)| sn);
    let bb = bb.iter().map(|(_, sn)| sn);
    aa.eq(bb)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use process::System;

    use crate::Snapshot;

    #[test]
    fn basic_usage() {
        let sys = System::with_procs(16);
        let snapshot = Arc::new(Snapshot::new(&sys, 0));

        #[allow(clippy::needless_collect)]
        let hs: Vec<_> = sys
            .procs()
            .iter()
            .map(|p| {
                let snapshot = Arc::clone(&snapshot);
                p.run(move |pid| {
                    thread::sleep(Duration::from_millis(100));
                    snapshot.update(pid, pid.as_usize());
                    let ss = snapshot.snapshot(pid);
                    snapshot.update(pid, ss[pid]);
                })
            })
            .collect();
        hs.into_iter().for_each(|h| h.join().unwrap());

        let num_procs = sys.num_procs();
        sys.procs()[0]
            .run(move |pid| {
                let ss = snapshot.snapshot(pid);
                assert_eq!(&*ss, &*(0..num_procs).collect::<Vec<_>>());
            })
            .join()
            .unwrap();
    }
}
